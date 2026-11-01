//! The observer's two transports, FUSE over io_uring and the classic
//! `/dev/fuse` loop, against each other.
//!
//! `transports_log_identically` runs the same workloads on a fresh ring
//! mount and a fresh classic mount and asserts the step saw the same thing
//! (stdout, including a stat of every entry: size, mode and times) and the
//! observer logged the same accesses, checks both serve content-derived file
//! times, then runs the tiny Go
//! module's `go test ./...` through the CLI on each and asserts identical
//! manifests. `per_request_cost` times the transports against each other.
//! The ring's failure paths are in ring_faults.rs (`fault-injection`).
//!
//! Every test here needs the ring (Linux, root, `fuse.enable_uring=Y`, a
//! kernel whose FUSE minor the ring transport is tested against) and is
//! #[ignore]d: `cargo test -p quod-cli --test transports -- --ignored`. On a
//! host without the ring they fail saying so; none compares classic with
//! classic.
mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use common::*;
use quod_cas::{Cas, Hash, Tree};
use quod_fs::{ObserverStats, Options, Transport};
use quod_ledger::Ledger;

/// A tree with every shape the observer serves: files (one large enough
/// for several reads), nested and empty directories, symlinks to a file, a
/// directory and nothing, and a directory big enough that listing it takes
/// several readdir requests at increasing offsets.
fn tree_dir(dir: &Path) {
    let w = |p: &str, b: &[u8]| {
        let p = dir.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b).unwrap();
    };
    w("go.mod", b"module x\n");
    w("pkg/cmd/root.go", b"package cmd\n");
    w("pkg/cmd/exec.sh", b"#!/bin/sh\n");
    w(
        "pkg/cmd/big.bin",
        &(0..3u32 << 20).map(|i| (i % 251) as u8).collect::<Vec<_>>(),
    );
    w("a/b/c/d.txt", b"deep\n");
    fs::create_dir_all(dir.join("empty")).unwrap();
    for i in 0..700 {
        w(&format!("big/file-with-a-longish-name-{i:04}.txt"), b"x");
    }
    symlink("pkg/cmd/root.go", dir.join("link")).unwrap();
    symlink("pkg", dir.join("dirlink")).unwrap();
    symlink("nope", dir.join("dangling")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        dir.join("pkg/cmd/exec.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
}

const WORKLOADS: &[(&str, &str)] = &[
    (
        "find and cat",
        "find . -type f -exec cat {} + | cksum; find . | sort | cksum",
    ),
    ("ls", "ls pkg/cmd; ls empty"),
    ("ls -lA", "ls -lA . pkg pkg/cmd"),
    (
        "stat of missing paths",
        "stat pkg/nope.go nope/deeper a/b/zz 2>&1; true",
    ),
    (
        "symlinks",
        "readlink link dirlink dangling; cat link; ls dirlink; cat dangling 2>&1; stat -c '%F %s' link dirlink; true",
    ),
    (
        "readdir offsets",
        "ls -f big | cksum; ls big | wc -l; ls big | tail -n 2",
    ),
    ("statfs", "stat -f -c '%c %b %S %l' ."),
    (
        // What a stat shows of every served entry (lstat: symlinks are not
        // followed): size, raw mode, permissions, nlink, and the three times
        // to the nanosecond. The ring encodes attributes itself, so this is
        // where its encoder and fuser's could drift apart.
        "stat of every entry",
        "find . -exec stat -c '%n %s %f %a %h %.9X %.9Y %.9Z' {} + | sort",
    ),
    (
        "reads at offsets",
        "dd if=pkg/cmd/big.bin bs=4096 skip=300 count=3 2>/dev/null | cksum; tail -c 100 pkg/cmd/big.bin | cksum; head -c 1 a/b/c/d.txt",
    ),
];

struct Fixture {
    tmp: tempfile::TempDir,
    cas: Cas,
    tree: Tree,
}

/// Runs `f` on its own thread and fails the test if it takes past a
/// minute: a hang is a failure, not a stuck test run.
fn within<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let limit = std::time::Duration::from_secs(60);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(limit)
        .unwrap_or_else(|_| panic!("{what} still running after {limit:?}: a hang"))
}

fn ring_needed(what: &str, e: impl std::fmt::Display) -> ! {
    panic!(
        "FUSE over io_uring is unavailable here ({e}), so {what} cannot run: it compares the ring with the classic transport"
    )
}

impl Fixture {
    fn new() -> Fixture {
        assert!(
            enabled(),
            "needs Linux and QUOD_INTEGRATION=1 (scripts/dev.sh)"
        );
        let tmp = tempfile::tempdir().unwrap();
        tree_dir(&tmp.path().join("src"));
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let tree = cas.import_dir(&tmp.path().join("src")).unwrap();
        Fixture { tmp, cas, tree }
    }

    /// Mounts fresh, runs `script` there, unmounts: (stdout, log, stats).
    fn run(
        &self,
        name: &str,
        transport: Transport,
        passthrough: bool,
        script: &str,
    ) -> (String, Vec<quod_manifest::Access>, ObserverStats) {
        let mnt = self.tmp.path().join(name);
        fs::create_dir_all(&mnt).unwrap();
        let opts = Options {
            transport,
            passthrough,
            ..Options::default()
        };
        let (tree, cas, at) = (self.tree.clone(), self.cas.clone(), mnt.clone());
        let served = within(&format!("serving {name}"), move || {
            quod_fs::serve_with(&tree, &cas, &at, &opts).map_err(|e| e.to_string())
        })
        .unwrap_or_else(|e| ring_needed("transports_log_identically", e));
        let sh = script.to_string();
        let out = within(&format!("{name}: {script}"), move || {
            Command::new("sh")
                .arg("-c")
                .arg(&sh)
                .current_dir(&mnt)
                .output()
                .unwrap()
        });
        assert!(
            out.status.success(),
            "{script}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stats = served.stats();
        let log = within(&format!("unmounting {name}"), move || served.unmount()).unwrap();
        (String::from_utf8(out.stdout).unwrap(), log, stats)
    }
}

#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y); run with --ignored"]
fn transports_log_identically() {
    let f = Fixture::new();
    let mut n = 0;
    for passthrough in [true, false] {
        for (name, script) in WORKLOADS {
            n += 1;
            let (ring_out, ring_log, rs) =
                f.run(&format!("r{n}"), Transport::Uring, passthrough, script);
            let (classic_out, classic_log, cs) =
                f.run(&format!("c{n}"), Transport::Classic, passthrough, script);
            eprintln!("{name} (passthrough {passthrough}): ring {rs:?}\n  classic {cs:?}");
            // The ring half really ran on the ring, the classic half did not.
            assert!(rs.uring && !rs.uring_fallback, "{name}: {rs:?}");
            // The queue threads run at the nice value asked for (root).
            assert_eq!(rs.ring_nice, Some(Options::default().uring_nice), "{name}");
            assert!(!rs.ring_nice_denied, "{name}");
            assert!(
                rs.ring_requests > 0 && rs.device_requests == 0,
                "{name}: {rs:?}"
            );
            assert!(!cs.uring && cs.ring_requests == 0, "{name}: {cs:?}");
            assert_eq!(
                ring_out, classic_out,
                "{name}: the step saw different things"
            );
            assert_eq!(
                ring_log, classic_log,
                "{name}: the transports logged differently"
            );
            assert!(!ring_log.is_empty(), "{name}: nothing logged");
            assert_eq!(rs.passthrough, cs.passthrough, "{name}");
            if *name == "find and cat" {
                if passthrough {
                    assert!(rs.passthrough, "{rs:?}");
                } else {
                    // Userspace reads: the ring's READ path served 3 MiB.
                    assert!(rs.read > 0 && rs.read_bytes >= 3 << 20, "{rs:?}");
                    assert!(cs.read > 0 && cs.read_bytes >= 3 << 20, "{cs:?}");
                }
            }
        }
    }

    // Both transports serve the times `CasFs::attr` chose: a file's derived
    // from its content, directories and symlinks fixed.
    let blob = |p: &str| {
        f.tree
            .iter()
            .find(|e| e.p == p)
            .and_then(|e| e.h)
            .unwrap_or_else(|| panic!("{p} is not a file of the tree"))
    };
    let at = |t: std::time::SystemTime| {
        let d = t.duration_since(std::time::UNIX_EPOCH).unwrap();
        format!("{}.{:09}", d.as_secs(), d.subsec_nanos())
    };
    let want = format!(
        "go.mod 9 {m} {m} {m}\npkg/cmd/root.go 12 {r} {r} {r}\npkg 0.000000000\nlink 0.000000000\n",
        m = at(quod_fs::content_mtime(&blob("go.mod"))),
        r = at(quod_fs::content_mtime(&blob("pkg/cmd/root.go"))),
    );
    let script =
        "stat -c '%n %s %.9X %.9Y %.9Z' go.mod pkg/cmd/root.go; stat -c '%n %.9Y' pkg link";
    for (name, transport) in [("rt", Transport::Uring), ("ct", Transport::Classic)] {
        let (out, _, _) = f.run(name, transport, true, script);
        assert_eq!(out, want, "{transport:?}: served times");
    }

    // go test on the tiny module, through the CLI, once per transport.
    let env = Env::new();
    let mut manifests = Vec::new();
    for (transport, want) in [("uring", "fuse-uring"), ("classic", "fuse")] {
        let state = env.tmp.path().join(format!("state-{transport}"));
        let o = env.quod_in(
            &state,
            &[
                "run",
                "--repo",
                env.repo.to_str().unwrap(),
                "--step",
                "go test ./...",
            ],
            &[("QUOD_FUSE_TRANSPORT", transport)],
        );
        if o.code == 2 && transport == "uring" {
            ring_needed("transports_log_identically", &o.stderr);
        }
        assert!(o.executed() && o.code == 0, "{transport}: {}", o.stderr);
        let run = last_event(&state, "run").unwrap();
        assert_eq!(run["transport"], want, "{run}");
        let detail = if transport == "uring" {
            format!("{want}+nice{}+", Options::default().uring_nice)
        } else {
            format!("{want}+")
        };
        assert!(
            run["observer"].as_str().unwrap().starts_with(&detail),
            "{run}"
        );
        let obs = last_event(&state, "observer").unwrap();
        if transport == "uring" {
            assert!(obs["ring_requests"].as_u64().unwrap() > 0, "{obs}");
            assert_eq!(obs["device_requests"], 0, "{obs}");
        }
        let ledger = Ledger::open(&state.join("ledger.sqlite")).unwrap();
        let row = ledger
            .run(run["run_id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(row.transport.as_deref(), Some(want));
        let key: Hash = run["step_key"].as_str().unwrap().parse().unwrap();
        let m = ledger.candidates(&key, 1).unwrap().remove(0);
        eprintln!("go test over {want}: {} inputs", m.entries.len());
        manifests.push(m);
    }
    assert!(manifests[0].entries.len() > 10);
    assert_eq!(
        manifests[0], manifests[1],
        "go test recorded different manifests over the two transports"
    );
}

/// This process's CPU time so far, (user, system) in ms, from
/// /proc/self/stat (clock ticks at 100 Hz): the observer's threads, not the
/// probe's child processes.
fn own_cpu_ms() -> (u64, u64) {
    let stat = fs::read_to_string("/proc/self/stat").unwrap();
    let after = &stat[stat.rfind(')').unwrap() + 2..];
    let f: Vec<&str> = after.split(' ').collect();
    // Fields 14 and 15 of the line; the slice starts at field 3.
    (
        f[11].parse::<u64>().unwrap() * 10,
        f[12].parse::<u64>().unwrap() * 10,
    )
}

fn loadavg() -> String {
    fs::read_to_string("/proc/loadavg")
        .unwrap_or_default()
        .split(' ')
        .next()
        .unwrap_or("?")
        .to_string()
}

/// Per-request cost of the real observer on each transport: the spike
/// repo's tree (QUOD_SPIKE_REPO) served, fresh mount per probe, 10k
/// module-cache files stat'ed, cat'ed and listed, serially and from 18
/// parallel processes, rounds alternating the transport order. Prints raw
/// numbers (wall, requests, the observer threads' CPU, load) and asserts
/// only that both transports logged the same accesses.
/// `QUOD_SPIKE_REPO=... cargo test --release -p quod-cli --test transports per_request_cost -- --ignored --nocapture`
#[test]
#[ignore = "needs FUSE over io_uring and QUOD_SPIKE_REPO; run with --ignored"]
fn per_request_cost() {
    let repo = std::env::var("QUOD_SPIKE_REPO").expect("set QUOD_SPIKE_REPO");
    let rounds: usize = std::env::var("QUOD_PROBE_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let tmp = tempfile::tempdir().unwrap();
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas
        .import_dir(Path::new(&repo))
        .unwrap()
        .without_top_level(".git");
    let files: Vec<&str> = tree
        .iter()
        .filter(|e| {
            e.k == quod_cas::Kind::File
                && e.p.starts_with(".quod/gomod/")
                && !e.p.starts_with(".quod/gomod/cache/")
        })
        .take(10_000)
        .map(|e| e.p.as_str())
        .collect();
    assert_eq!(files.len(), 10_000, "the module cache has too few files");
    let list = tmp.path().join("files.txt");
    fs::write(&list, files.join("\n") + "\n").unwrap();
    let l = list.display();
    let probes = [
        (
            "stat, serial",
            format!("xargs -d '\\n' -a {l} stat -c %s >/dev/null"),
        ),
        (
            "stat, 18 parallel",
            format!("xargs -d '\\n' -a {l} -P 18 -n 100 stat -c %s >/dev/null"),
        ),
        (
            "cat, serial",
            format!("xargs -d '\\n' -a {l} cat >/dev/null"),
        ),
        (
            "cat, 18 parallel",
            format!("xargs -d '\\n' -a {l} -P 18 -n 100 cat >/dev/null"),
        ),
        ("ls -R", "ls -R .quod/gomod >/dev/null".to_string()),
    ];
    println!(
        "per-request cost: {} files of {repo}, {rounds} rounds, nproc {}",
        files.len(),
        std::thread::available_parallelism().map_or(0, |n| n.get())
    );
    let mut n = 0;
    for round in 1..=rounds {
        for (name, script) in &probes {
            // (label, transport, pinned, nice): the default ring, the
            // classic transport, and the ring without each of its two
            // scheduling choices.
            let mut variants = vec![
                (
                    "uring",
                    Transport::Uring,
                    true,
                    Options::default().uring_nice,
                ),
                ("classic", Transport::Classic, true, 0),
                (
                    "unpinned",
                    Transport::Uring,
                    false,
                    Options::default().uring_nice,
                ),
                ("nice0", Transport::Uring, true, 0),
            ];
            variants.rotate_left(round % 4);
            let mut logs = Vec::new();
            for (label, transport, uring_pin, uring_nice) in variants {
                n += 1;
                let mnt = tmp.path().join(format!("m{n}"));
                fs::create_dir_all(&mnt).unwrap();
                let opts = Options {
                    transport,
                    uring_pin,
                    uring_nice,
                    ..Options::default()
                };
                let served = quod_fs::serve_with(&tree, &cas, &mnt, &opts)
                    .unwrap_or_else(|e| ring_needed("per_request_cost", e));
                let load = loadavg();
                let cpu0 = own_cpu_ms();
                let t = Instant::now();
                let st = Command::new("sh")
                    .arg("-c")
                    .arg(script)
                    .current_dir(&mnt)
                    .status()
                    .unwrap();
                let wall = t.elapsed().as_secs_f64();
                let cpu1 = own_cpu_ms();
                assert!(st.success(), "{name}");
                let s = served.stats();
                let sch = served.sched();
                logs.push(served.unmount().unwrap());
                println!(
                    "probe round={round} {name:<18} {label:<8} wall={wall:.3}s lookup={} open={} readdir={} getattr={} ring={} device={} batches={} full_batches={} ring_serve_us={} contended={} observer_cpu_ms={}/{} threads={} run_ms={} runq_wait_ms={} slices={} load={load}",
                    s.lookup,
                    s.open,
                    s.readdir,
                    s.getattr,
                    s.ring_requests,
                    s.device_requests,
                    s.ring_batches,
                    s.ring_full_batches,
                    s.ring_serve_us,
                    s.lock_contended,
                    cpu1.0 - cpu0.0,
                    cpu1.1 - cpu0.1,
                    sch.threads,
                    sch.run_ms,
                    sch.wait_ms,
                    sch.slices,
                );
            }
            assert!(
                logs.windows(2).all(|w| w[0] == w[1]),
                "{name}: the transports logged differently"
            );
        }
    }
}
