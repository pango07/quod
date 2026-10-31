//! T09: the execute path end to end.
mod common;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, UNIX_EPOCH};

use common::*;
use sp_cas::Hash;

#[test]
fn execute_records_one_manifest_and_one_result() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let o = env.run_step("go test ./...", &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.executed());
    assert!(o.stderr.contains("no manifests for this step key"));
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.contains("example.com/tiny/calc"), "{stdout}");

    let ledger = rusqlite_count(&env, "SELECT count(*) FROM manifests");
    assert_eq!(ledger, 1);
    assert_eq!(rusqlite_count(&env, "SELECT count(*) FROM results"), 1);
    let key = scalar(&env, "SELECT step_key FROM manifests");
    let i = env.stillpass(&["inspect", &key]);
    assert_eq!(i.code, 0);
    let listing = String::from_utf8_lossy(&i.stdout);
    assert!(listing.contains("1 manifest(s)"), "{listing}");
    let run = scalar(&env, "SELECT id FROM runs");
    let r = env.stillpass(&["inspect", &run]);
    assert!(String::from_utf8_lossy(&r.stdout).contains("\"exit_code\": 0"));

    // A failing step is recorded with its exit code, not as a Stillpass error.
    let f = env.run_step("echo boom >&2; exit 3", &[]);
    assert_eq!(f.code, 3);
    assert!(f.executed() && !f.stderr.contains("ERROR"));
}

#[test]
fn stillpass_errors_exit_2() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let o = env.run_step(
        "true",
        &["--commit", "0000000000000000000000000000000000000000"],
    );
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("STILLPASS  ERROR"), "{}", o.stderr);
}

/// The sealed module cache is a snapshot of the imported tree: a change to
/// the repo's copy while the step runs is neither seen by the step nor
/// recorded, and the next run sees the changed tree as a miss.
#[test]
fn sealed_module_cache_is_a_snapshot() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let modfile = ".stillpass/gomod/example.com/m@v1/a.txt";
    env.write(modfile, "one\n");
    std::fs::create_dir_all(env.repo.join(".stillpass/gomod/cache")).unwrap();
    // The step reads the file well after the test has had time to change it,
    // and writes into the carve-out, which must stay writable.
    let step = format!("sleep 3; cat {modfile}; touch .stillpass/gomod/cache/lock");
    let child = Command::new(env!("CARGO_BIN_EXE_stillpass"))
        .arg("--state")
        .arg(env.state())
        .args(["run", "--repo", env.repo.to_str().unwrap(), "--step", &step])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The snapshot is built (renamed into place) after the import and
    // before the step starts.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snapshots = std::fs::read_dir(env.state().join("cas/snapshots"))
            .map(|d| {
                d.flatten()
                    .filter(|r| r.file_type().is_ok_and(|t| t.is_dir()))
                    .filter(|r| r.path().join(".complete").is_file())
                    .count()
            })
            .unwrap_or(0);
        if snapshots > 0 {
            break;
        }
        assert!(Instant::now() < deadline, "no sealed snapshot appeared");
        std::thread::sleep(Duration::from_millis(50));
    }
    env.write(modfile, "two\n");
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(
        stderr.contains("STILLPASS  EXECUTED") && !stderr.contains("ERROR"),
        "{stderr}"
    );
    assert_eq!(
        out.stdout, b"one\n",
        "the step read the snapshot, not the live file"
    );
    let json = scalar(&env, "SELECT json FROM manifests");
    let want = format!(
        "\"h\":\"{}\",\"k\":\"f\",\"m\":420,\"p\":\"{modfile}\"",
        Hash::of(b"one\n")
    );
    assert!(
        json.contains(&want),
        "the manifest records what the step saw: {json}"
    );
    assert!(
        json.contains("\"p\":\".stillpass/gomod/example.com/m@v1\""),
        "{json}"
    );

    // The changed tree is a miss that names the file, and the step sees it:
    // another snapshot, since the subtree differs.
    let o = env.run_step(&step, &[]);
    assert!(
        o.executed() && o.stderr.contains(&format!("differed at {modfile}")),
        "{}",
        o.stderr
    );
    assert_eq!(o.stdout, b"two\n");
    // A fresh key on the same tree reuses that snapshot.
    let o = env.run_step(&step, &["--env", "STILLPASS_T=1"]);
    assert!(o.executed() && o.stdout == b"two\n", "{}", o.stderr);
    let events = std::fs::read_to_string(env.state().join("events.jsonl")).unwrap();
    let snapshots: Vec<bool> = events
        .lines()
        .filter(|l| l.contains("\"message\":\"snapshot\""))
        .map(|l| l.contains("\"reused\":true"))
        .collect();
    assert_eq!(snapshots, [false, false, true], "{events}");
    assert_eq!(
        std::fs::read_dir(env.state().join("cas/snapshots"))
            .unwrap()
            .flatten()
            .filter(|e| e.file_type().unwrap().is_dir())
            .count(),
        2
    );
}

/// Rootfs mode with the real rootfs (STILLPASS_ROOTFS names an imported
/// rootfs): the key's image is the rootfs on this platform, the host's
/// userland is invisible, writes anywhere outside the workspace (/tmp
/// included) are recorded as root outputs, and the same tree replays.
#[test]
fn rootfs_mode_pins_the_image_and_records_root_outputs() {
    if !enabled() {
        return;
    }
    let Ok(rootfs) = std::env::var("STILLPASS_ROOTFS") else {
        eprintln!("STILLPASS_ROOTFS not set; skipping");
        return;
    };
    let env = Env::new();
    let step = "cat /etc/hostname; for p in /etc/fuse.conf /src /work /.complete /.dockerenv /sys/kernel; do test -e $p && echo LEAK $p; done; \
                go version; git --version >/dev/null; \
                echo marker > /opt/stillpass-marker; mkdir -p /home/runner/.cache && echo c > /home/runner/.cache/f; \
                rm /etc/hosts; echo w > /workspace/out.txt; cat /etc/hostname > /tmp/x; true";
    let o = env.run_step(step, &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(
        o.executed() && o.stderr.contains("image rootfs b3:"),
        "{}",
        o.stderr
    );
    assert!(
        o.stderr.contains("5 outputs outside the workspace"),
        "{}",
        o.stderr
    );
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.starts_with("stillpass\ngo version go"), "{stdout}");
    assert!(!stdout.contains("LEAK"), "{stdout}");
    let result = scalar(&env, "SELECT json FROM results");
    for want in [
        "\"root_outputs\":[",
        "\"k\":\"x\",\"p\":\"etc/hosts\"",
        "\"k\":\"d\",\"p\":\"home/runner\"",
        "\"k\":\"d\",\"p\":\"home/runner/.cache\"",
        "\"k\":\"f\",\"m\":420,\"p\":\"home/runner/.cache/f\"",
        "\"k\":\"f\",\"m\":420,\"p\":\"opt/stillpass-marker\"",
        "\"p\":\"out.txt\"",
    ] {
        assert!(result.contains(want), "{want} not in {result}");
    }
    assert!(!result.contains("stillpass-started"), "{result}");
    // /tmp is scratch: its leftovers are not outputs, nor in the store.
    assert!(!result.contains("\"p\":\"tmp/"), "{result}");
    let key = scalar(&env, "SELECT step_key FROM manifests");
    let i = env.stillpass(&["inspect", &key]);
    let text = String::from_utf8_lossy(&i.stdout);
    assert!(
        text.contains("\"kind\": \"rootfs\"")
            && text.contains(&format!("\"digest\": \"{rootfs}\""))
            && text.contains("\"cpu_features\": ")
            && text.contains("\"page_size\": "),
        "{text}"
    );
    // The host class, read in the sandbox and outside it.
    let doc = scalar(
        &env,
        &format!("SELECT json FROM step_keys WHERE step_key = '{key}'"),
    );
    let doc: serde_json::Value = serde_json::from_str(&doc).unwrap();
    let host: std::collections::BTreeMap<String, String> = doc["image"]["platform"]["host"]
        .as_array()
        .unwrap()
        .iter()
        .map(|kv| {
            (
                kv[0].as_str().unwrap().into(),
                kv[1].as_str().unwrap().into(),
            )
        })
        .collect();
    for (k, want) in [
        ("status Umask", Some("0022")),
        ("status SigIgn", Some("0000000000000000")),
        ("status SigBlk", Some("0000000000000000")),
        ("status NoNewPrivs", Some("1")),
        ("nice", Some("0")),
        ("rlimit NOFILE", None),
        ("rlimit NPROC", None),
        ("memory", None),
        ("kernel config", None),
        ("cgroup pids.max", None),
        ("cgroup memory.max", None),
        ("binfmt_misc", None),
        ("fs store", None),
        ("sandbox", None),
        ("root tmpfs", Some(ROOT_TMPFS)),
    ] {
        let v = host
            .get(k)
            .unwrap_or_else(|| panic!("{k} not in the host class: {host:?}"));
        if let Some(w) = want {
            assert_eq!(v, w, "{k}");
        }
    }
    assert!(
        host["memory"].ends_with(" kB"),
        "exact MemTotal: {}",
        host["memory"]
    );
    assert!(
        !host["cpuinfo"].to_ascii_lowercase().contains("bogomips"),
        "{}",
        host["cpuinfo"]
    );
    // A constant, capped only by a lower hard limit on the host.
    let nproc: Vec<u64> = host["rlimit NPROC"]
        .split(' ')
        .map(|x| x.parse().unwrap())
        .collect();
    assert!(nproc[0] <= nproc[1] && nproc[1] <= 16384, "{nproc:?}");
    let h = env.run_step(step, &[]);
    assert!(h.hit() && h.stdout == o.stdout, "{}", h.stderr);
    // The host pin is another key with no proofs; the flag beats the variable.
    let hm = env.run_step("true", &["--host-root"]);
    assert!(
        hm.executed()
            && hm.stderr.contains("image host fingerprint")
            && hm.stderr.contains("no manifests for this step key"),
        "{}",
        hm.stderr
    );
    let list = env.stillpass(&["rootfs", "list"]);
    assert!(String::from_utf8_lossy(&list.stdout).contains(&rootfs));
}

#[test]
fn an_unknown_rootfs_is_a_stillpass_error() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let o = env.run_step("true", &["--rootfs", &format!("b3:{}", "0".repeat(64))]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("not in the store"), "{}", o.stderr);
    let o = env.run_step("true", &["--rootfs", "latest"]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("neither a rootfs digest"), "{}", o.stderr);
}

/// A parent with umask 077, SIGINT ignored and a stray descriptor gets the
/// same key, the same outputs (modes included) and the same stdout as a
/// plain parent, and replays the plain parent's proof: the step's process
/// state is fixed by the sandbox, not inherited.
#[test]
fn a_parents_process_state_changes_nothing() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let step = "umask; while read -r k v rest; do case $k in SigIgn:|SigBlk:) echo \"$k $v\";; esac; done < /proc/self/status; \
                echo x > out.txt; mkdir -p outdir; echo y > outdir/f; ulimit -n";
    let hostile = |extra: &[&str]| {
        let repo = env.repo.to_str().unwrap().to_string();
        let state = env.state();
        let mut args: Vec<String> = vec![
            "--state".into(),
            state.to_str().unwrap().into(),
            "run".into(),
            "--repo".into(),
            repo,
            "--step".into(),
            step.into(),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        let out = Command::new("sh")
            .arg("-c")
            .arg("umask 077; trap '' INT HUP; exec 7</dev/null; ulimit -S -n 512; exec \"$0\" \"$@\"")
            .arg(env!("CARGO_BIN_EXE_stillpass"))
            .args(&args)
            .output()
            .unwrap();
        Out {
            code: out.status.code().unwrap_or(-1),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).into(),
        }
    };
    let a = env.run_step(step, &["--env", "RUN=a"]);
    assert_eq!(a.code, 0, "{}", a.stderr);
    let b = hostile(&["--env", "RUN=b"]);
    assert_eq!(b.code, 0, "{}", b.stderr);
    assert!(b.executed(), "{}", b.stderr);
    assert_eq!(
        String::from_utf8_lossy(&b.stdout),
        String::from_utf8_lossy(&a.stdout)
    );
    assert!(
        String::from_utf8_lossy(&a.stdout)
            .starts_with("0022\nSigBlk: 0000000000000000\nSigIgn: 0000000000000000\n1024\n"),
        "{}",
        String::from_utf8_lossy(&a.stdout)
    );
    // Same outputs, modes included (mode 420 is 0644).
    // Results in the order recorded: run a, then run b.
    let outputs = |n: usize| {
        let r = scalar(
            &env,
            &format!("SELECT json FROM results ORDER BY created_at LIMIT 1 OFFSET {n}"),
        );
        let v: serde_json::Value = serde_json::from_str(&r).unwrap();
        v["outputs"].clone()
    };
    assert_eq!(outputs(0), outputs(1));
    assert!(outputs(0).to_string().contains(r#""m":420,"p":"out.txt""#));
    // And the hostile parent replays the plain parent's proof.
    let c = hostile(&["--env", "RUN=a"]);
    assert!(c.hit() && c.stdout == a.stdout, "{}", c.stderr);
}

/// The host class is read the way the step sees the host, in a sandbox
/// with a fresh procfs, never from the caller's /proc. Where the caller's
/// /proc files are overmounts (lxcfs virtualizes meminfo and cpuinfo in
/// containers that way), the kernel refuses a fresh procfs in a user
/// namespace, so neither the probe nor a step can run: a Stillpass error
/// and nothing recorded, never a key built from the virtualized values.
#[test]
fn a_virtualized_proc_is_never_keyed() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let fake = env.tmp.path().join("fake");
    std::fs::create_dir_all(&fake).unwrap();
    std::fs::write(fake.join("meminfo"), "MemTotal:       1024 kB\n").unwrap();
    let script = format!(
        "mount --bind {f}/meminfo /proc/meminfo && grep MemTotal /proc/meminfo && exec \"$0\" \"$@\"",
        f = fake.display()
    );
    let out = Command::new("unshare")
        .args(["-m", "sh", "-c", &script])
        .arg(env!("CARGO_BIN_EXE_stillpass"))
        .arg("--state")
        .arg(env.state())
        .args([
            "run",
            "--repo",
            env.repo.to_str().unwrap(),
            "--step",
            "true",
        ])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !stdout.starts_with("MemTotal:       1024 kB") {
        eprintln!("cannot overmount /proc here (needs root and unshare); skipping: {stdout}");
        return;
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("probe the host class in the sandbox"),
        "{stderr}"
    );
    let results = Command::new("sqlite3")
        .arg(env.state().join("ledger.sqlite"))
        .arg("SELECT count(*) FROM results")
        .output()
        .unwrap();
    let n = String::from_utf8_lossy(&results.stdout).trim().to_string();
    assert!(n.is_empty() || n == "0", "recorded {n} results");
}

/// rootfs.rs `ROOT_TMPFS`, as the key records it.
const ROOT_TMPFS: &str = "size=4096m,nr_inodes=1048576,mode=0755";

/// Served files carry a content-derived mtime, the same in the observed
/// tree and in a sealed snapshot; directories a fixed one. A cache keyed by
/// mtime therefore sees a change exactly when the content changes.
#[test]
fn served_mtimes_follow_content() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    env.write("same1.txt", "x\n");
    env.write("same2.txt", "x\n");
    env.write("other.txt", "y\n");
    let sealed = ".stillpass/gomod/example.com/m@v1";
    env.write(&format!("{sealed}/same3.txt"), "x\n");
    let step =
        format!("stat -c '%n %.9Y' same1.txt same2.txt other.txt calc {sealed}/same3.txt {sealed}");
    let o = env.run_step(&step, &[]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    let times: std::collections::BTreeMap<&str, &str> =
        out.lines().filter_map(|l| l.split_once(' ')).collect();
    let d = sp_fs::content_mtime(&Hash::of(b"x\n"))
        .duration_since(UNIX_EPOCH)
        .unwrap();
    let want = format!("{}.{:09}", d.as_secs(), d.subsec_nanos());
    assert_eq!(times["same1.txt"], want, "{out}");
    assert_eq!(times["same2.txt"], want, "{out}");
    assert_eq!(times[format!("{sealed}/same3.txt").as_str()], want, "{out}");
    assert_ne!(times["other.txt"], want, "{out}");
    assert_eq!(times["calc"], "0.000000000", "{out}");
    assert_eq!(times[sealed], "0.000000000", "{out}");
}

fn scalar(env: &Env, sql: &str) -> String {
    let out = std::process::Command::new("sqlite3")
        .arg(env.state().join("ledger.sqlite"))
        .arg(sql)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn rusqlite_count(env: &Env, sql: &str) -> u64 {
    scalar(env, sql).parse().unwrap()
}
