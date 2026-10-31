//! T09: the execute path end to end.
mod common;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
        let snapshots = std::fs::read_dir(env.state().join("snapshots"))
            .map(|d| {
                d.flatten()
                    .filter(|r| !r.file_name().to_string_lossy().starts_with("tmp."))
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
        std::fs::read_dir(env.state().join("snapshots"))
            .unwrap()
            .count(),
        2
    );
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
