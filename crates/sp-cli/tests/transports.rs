//! The observer's two transports, FUSE over io_uring and the classic
//! `/dev/fuse` loop, against each other.
//!
//! `transports_log_identically` runs the same workloads on a fresh ring
//! mount and a fresh classic mount and asserts the step saw the same thing
//! (stdout) and the observer logged the same accesses, then runs the tiny Go
//! module's `go test ./...` through the CLI on each and asserts identical
//! manifests. The other tests run the CLI with the ring's failure hooks: a
//! ring that cannot come up is a recorded fallback, a ring that dies under
//! the step is a fast Stillpass error with nothing recorded.
//!
//! Every test here needs the ring (Linux, root, `fuse.enable_uring=Y`, a
//! kernel whose FUSE minor the ring transport is tested against) and is
//! #[ignore]d: `cargo test -p sp-cli --test transports -- --ignored`. On a
//! host without the ring they fail saying so; none compares classic with
//! classic.
mod common;

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::*;
use sp_cas::{Cas, Hash, Tree};
use sp_fs::{ObserverStats, Options, Transport};
use sp_ledger::Ledger;

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
        "reads at offsets",
        "dd if=pkg/cmd/big.bin bs=4096 skip=300 count=3 2>/dev/null | cksum; tail -c 100 pkg/cmd/big.bin | cksum; head -c 1 a/b/c/d.txt",
    ),
];

struct Fixture {
    tmp: tempfile::TempDir,
    cas: Cas,
    tree: Tree,
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
            "needs Linux and STILLPASS_INTEGRATION=1 (scripts/dev.sh)"
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
    ) -> (String, Vec<sp_manifest::Access>, ObserverStats) {
        let mnt = self.tmp.path().join(name);
        fs::create_dir_all(&mnt).unwrap();
        let opts = Options {
            transport,
            passthrough,
            ..Options::default()
        };
        let served = sp_fs::serve_with(&self.tree, &self.cas, &mnt, &opts)
            .unwrap_or_else(|e| ring_needed("transports_log_identically", e));
        let out = Command::new("sh")
            .arg("-c")
            .arg(script)
            .current_dir(&mnt)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{script}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stats = served.stats();
        let log = served.unmount().unwrap();
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

    // go test on the tiny module, through the CLI, once per transport.
    let env = Env::new();
    let mut manifests = Vec::new();
    for (transport, want) in [("uring", "fuse-uring"), ("classic", "fuse")] {
        let state = env.tmp.path().join(format!("state-{transport}"));
        let o = env.stillpass_in(
            &state,
            &[
                "run",
                "--repo",
                env.repo.to_str().unwrap(),
                "--step",
                "go test ./...",
            ],
            &[("STILLPASS_FUSE_TRANSPORT", transport)],
        );
        if o.code == 2 && transport == "uring" {
            ring_needed("transports_log_identically", &o.stderr);
        }
        assert!(o.executed() && o.code == 0, "{transport}: {}", o.stderr);
        let run = last_event(&state, "run").unwrap();
        assert_eq!(run["transport"], want, "{run}");
        assert!(
            run["observer"]
                .as_str()
                .unwrap()
                .starts_with(&format!("{want}+")),
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

/// Runs the CLI with a ring failure hook, bounded: past `limit` the test
/// fails (a hang). Returns the output and how long the run took.
fn hooked(env: &Env, state: &Path, fault: &str, transport: Option<&str>) -> (Out, Duration) {
    let mut envs = vec![("STILLPASS_TEST_URING_FAULT", fault)];
    if let Some(t) = transport {
        envs.push(("STILLPASS_FUSE_TRANSPORT", t));
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let (repo, st, envs2): (String, std::path::PathBuf, Vec<(String, String)>) = (
        env.repo.to_str().unwrap().into(),
        state.to_path_buf(),
        envs.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    );
    let t = Instant::now();
    std::thread::spawn(move || {
        let out = Command::new(env!("CARGO_BIN_EXE_stillpass"))
            .arg("--state")
            .arg(&st)
            .args([
                "run",
                "--repo",
                &repo,
                "--step",
                "cat go.mod && go test ./...",
            ])
            .envs(envs2)
            .output()
            .unwrap();
        let _ = tx.send(out);
    });
    let out = rx
        .recv_timeout(Duration::from_secs(300))
        .unwrap_or_else(|_| panic!("STILLPASS_TEST_URING_FAULT={fault}: the run hung"));
    let o = Out {
        code: out.status.code().unwrap_or(-1),
        stdout: out.stdout,
        stderr: String::from_utf8_lossy(&out.stderr).into(),
    };
    eprintln!(
        "fault {fault} transport {transport:?}: exit {} in {:?}\n{}",
        o.code,
        t.elapsed(),
        o.stderr
    );
    (o, t.elapsed())
}

#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y); run with --ignored"]
fn a_ring_queue_that_dies_under_the_step_fails_the_run_fast() {
    assert!(enabled(), "needs Linux and STILLPASS_INTEGRATION=1");
    let env = Env::new();
    // The baseline: the same step on a healthy ring passes and is recorded.
    let ok_state = env.tmp.path().join("state-ok");
    let o = env.stillpass_in(
        &ok_state,
        &[
            "run",
            "--repo",
            env.repo.to_str().unwrap(),
            "--step",
            "cat go.mod && go test ./...",
        ],
        &[("STILLPASS_FUSE_TRANSPORT", "uring")],
    );
    if o.code == 2 {
        ring_needed("this test", &o.stderr);
    }
    assert!(o.executed() && o.code == 0, "{}", o.stderr);
    // Half the baseline's ring requests in, the failure lands inside go
    // test; with no count it lands on the first request after the mount
    // came up (the overlay mounting over it).
    let half = last_event(&ok_state, "observer").unwrap()["ring_requests"]
        .as_u64()
        .unwrap()
        / 2;
    assert!(half > 10, "the step sends too few requests to aim at");
    let mid = [format!("panic:{half}"), format!("error:{half}")];
    for fault in ["panic", "error", &mid[0], &mid[1]] {
        let state = env
            .tmp
            .path()
            .join(format!("state-{}", fault.replace(':', "-")));
        let (o, took) = hooked(&env, &state, fault, Some("uring"));
        if fault.contains(':') {
            // The step ran and failed; the observer's record of why is the
            // error the run reports.
            assert!(
                o.stderr.contains("FUSE over io_uring"),
                "{fault}: {}",
                o.stderr
            );
        }
        assert_eq!(
            o.code, 2,
            "{fault}: a dead observer must be a Stillpass error: {}",
            o.stderr
        );
        assert!(o.stderr.contains("STILLPASS  ERROR"), "{}", o.stderr);
        assert!(
            !o.executed(),
            "{fault}: reported as a step result: {}",
            o.stderr
        );
        assert!(took < Duration::from_secs(120), "{fault}: took {took:?}");
        let ledger = Ledger::open(&state.join("ledger.sqlite")).unwrap();
        assert!(
            ledger
                .results_by_transport(Some("fuse-uring"))
                .unwrap()
                .is_empty()
                && ledger
                    .results_by_transport(Some("fuse"))
                    .unwrap()
                    .is_empty()
                && ledger.results_by_transport(None).unwrap().is_empty(),
            "{fault}: a result was recorded"
        );
    }
}

#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y); run with --ignored"]
fn a_ring_that_cannot_come_up_is_a_recorded_fallback() {
    assert!(enabled(), "needs Linux and STILLPASS_INTEGRATION=1");
    let env = Env::new();
    // Forced, the ring's failure is the run's.
    let state = env.tmp.path().join("state-forced");
    let (o, _) = hooked(&env, &state, "register", Some("uring"));
    assert_eq!(o.code, 2, "{}", o.stderr);
    if !o.stderr.contains("REGISTER") {
        ring_needed("this test", &o.stderr);
    }
    // Auto: classic serves the whole mount, and the run says so.
    let state = env.tmp.path().join("state-auto");
    let (o, _) = hooked(&env, &state, "register", None);
    assert!(o.executed() && o.code == 0, "{}", o.stderr);
    let run = last_event(&state, "run").unwrap();
    assert_eq!(run["transport"], "fuse", "{run}");
    let observer = run["observer"].as_str().unwrap();
    assert!(observer.starts_with("fuse+uring-fallback"), "{run}");
    let ledger = Ledger::open(&state.join("ledger.sqlite")).unwrap();
    let row = ledger
        .run(run["run_id"].as_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.transport.as_deref(), row.observer.as_deref()),
        (Some("fuse"), Some(observer))
    );
    assert_eq!(ledger.results_by_transport(Some("fuse")).unwrap().len(), 1);
    let obs = last_event(&state, "observer").unwrap();
    assert_eq!(
        (
            obs["uring_fallback"].as_bool(),
            obs["ring_requests"].as_u64()
        ),
        (Some(true), Some(0)),
        "{obs}"
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
/// repo's tree (STILLPASS_SPIKE_REPO) served, fresh mount per probe, 10k
/// module-cache files stat'ed, cat'ed and listed, serially and from 18
/// parallel processes, rounds alternating the transport order. Prints raw
/// numbers (wall, requests, the observer threads' CPU, load) and asserts
/// only that both transports logged the same accesses.
/// `STILLPASS_SPIKE_REPO=... cargo test --release -p sp-cli --test transports per_request_cost -- --ignored --nocapture`
#[test]
#[ignore = "needs FUSE over io_uring and STILLPASS_SPIKE_REPO; run with --ignored"]
fn per_request_cost() {
    let repo = std::env::var("STILLPASS_SPIKE_REPO").expect("set STILLPASS_SPIKE_REPO");
    let rounds: usize = std::env::var("STILLPASS_PROBE_ROUNDS")
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
            e.k == sp_cas::Kind::File
                && e.p.starts_with(".stillpass/gomod/")
                && !e.p.starts_with(".stillpass/gomod/cache/")
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
        ("ls -R", "ls -R .stillpass/gomod >/dev/null".to_string()),
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
                let served = sp_fs::serve_with(&tree, &cas, &mnt, &opts)
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
