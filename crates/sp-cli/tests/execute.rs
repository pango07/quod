//! T09: the execute path end to end.
mod common;
use common::*;

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
