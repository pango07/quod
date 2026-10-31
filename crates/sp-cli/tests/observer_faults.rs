//! T7: a panic in the observer while it serves a step is a Stillpass error
//! (exit 2) that records nothing, aborts the step at once and leaves no
//! mount behind, on either transport. The panic is injected with
//! STILLPASS_FAULT, which only `fault-injection` builds read, so this
//! target requires the feature:
//! `cargo test -p sp-cli --features fault-injection --test observer_faults`.
//! The ring case needs FUSE over io_uring (root, `fuse.enable_uring=Y`) and
//! is #[ignore]d like the other ring tests (`-- --include-ignored`).

mod common;
use std::process::Command;
use std::time::{Duration, Instant};

use common::*;

/// Runs a step whose first read panics the observer, with `transport`
/// (`classic` or `uring`), and checks the run failed as Stillpass's own
/// error with nothing recorded and nothing left mounted. Returns stderr.
fn panic_run(transport: &str) -> String {
    let env = Env::new();
    let t = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_stillpass"))
        .arg("--state")
        .arg(env.state())
        .args([
            "run",
            "--repo",
            env.repo.to_str().unwrap(),
            "--step",
            "cat README.md; sleep 120",
        ])
        .env(sp_fs::FAULT_ENV, "observer-panic:README.md")
        .env("STILLPASS_FUSE_TRANSPORT", transport)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(
        stderr.contains("STILLPASS  ERROR") && stderr.contains("injected fault"),
        "{stderr}"
    );
    assert!(
        t.elapsed() < Duration::from_secs(60),
        "the sandbox was aborted, not left to sleep: {:?}",
        t.elapsed()
    );
    assert_eq!(env.manifests(), 0, "nothing recorded");
    let mounts = std::fs::read_to_string("/proc/self/mounts").unwrap();
    assert!(!mounts.contains(env.state().to_str().unwrap()), "{mounts}");
    // The next run is healthy.
    let o = env.run_step("cat README.md", &[]);
    assert!(o.executed() && o.code == 0, "{}", o.stderr);
    stderr
}

#[test]
fn t7_an_observer_panic_is_a_stillpass_error_and_records_nothing() {
    if !enabled() {
        return;
    }
    let stderr = panic_run("classic");
    assert!(
        stderr.contains("the observer failed while serving the step")
            && stderr.contains("lookup panicked"),
        "{stderr}"
    );
    println!(
        "T7 ok: an injected observer panic aborted the step and failed the run with exit 2 (classic)"
    );
}

#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y)"]
fn t7_a_ring_queue_panic_is_a_stillpass_error_and_records_nothing() {
    if !enabled() {
        return;
    }
    let stderr = panic_run("uring");
    assert!(
        stderr.contains("FUSE over io_uring") && stderr.contains("ring queue"),
        "{stderr}"
    );
    println!(
        "T7 ok: an injected panic in a ring queue aborted the step and failed the run with exit 2 (ring)"
    );
}
