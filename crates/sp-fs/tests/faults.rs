//! A panic in the observer while it serves a step, on either transport,
//! must not hang the step: the abort flag (the sandbox's kill switch) is up
//! before the step sees the failed request, every later request fails, and
//! unmount refuses to return a log. The fault is injected with
//! STILLPASS_FAULT (`fault-injection` builds only: this target requires
//! the feature), which the observer reads when it mounts, so each case runs
//! in a child process of this test binary.
//!
//! The classic case needs FUSE (STILLPASS_INTEGRATION=1). The ring case
//! needs FUSE over io_uring (root, `fuse.enable_uring=Y`) and is #[ignore]d
//! like the other ring tests: `cargo test -p sp-fs --features
//! fault-injection --test faults -- --include-ignored`.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::Ordering;

use sp_cas::Cas;
use sp_fs::{FsError, Options, Transport};

fn enabled() -> bool {
    let on = std::env::var("STILLPASS_INTEGRATION").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("needs STILLPASS_INTEGRATION=1; skipping");
    }
    on
}

fn sample(dir: &Path) {
    fs::create_dir_all(dir.join("pkg/cmd")).unwrap();
    fs::write(dir.join("go.mod"), "module x\n").unwrap();
    fs::write(dir.join("pkg/cmd/root.go"), "package cmd\n").unwrap();
    fs::write(dir.join("pkg/cmd/big.bin"), vec![7u8; 4096]).unwrap();
}

/// Re-runs `test` alone in a child with the fault armed; true when this
/// is the child (the caller then runs the body).
fn in_child(test: &str) -> bool {
    if std::env::var_os(sp_fs::FAULT_ENV).is_some() {
        return true;
    }
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture", "--include-ignored"])
        .env(sp_fs::FAULT_ENV, "observer-panic:pkg/cmd/root.go")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "child failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    false
}

/// Mounts the sample with `transport`, triggers the fault, and returns
/// what the step would see and what unmount reports.
fn fault_on(transport: Transport) -> (std::io::Error, FsError) {
    let tmp = tempfile::tempdir().unwrap();
    sample(&tmp.path().join("src"));
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas.import_dir(&tmp.path().join("src")).unwrap();
    let mnt = tmp.path().join("mnt");
    fs::create_dir_all(&mnt).unwrap();
    let served = sp_fs::serve_with(
        &tree,
        &cas,
        &mnt,
        &Options {
            transport,
            ..Options::default()
        },
    )
    .unwrap();
    let abort = served.abort_flag();
    assert!(!abort.load(Ordering::SeqCst));
    // Healthy before the fault.
    assert_eq!(
        fs::read_to_string(mnt.join("go.mod")).unwrap(),
        "module x\n"
    );
    let err = fs::metadata(mnt.join("pkg/cmd/root.go")).unwrap_err();
    assert!(
        abort.load(Ordering::SeqCst),
        "abort raised before the step saw the error ({err})"
    );
    // Nothing is served after it.
    assert!(fs::read(mnt.join("pkg/cmd/big.bin")).is_err());
    let unmounted = served
        .unmount()
        .expect_err("unmount must refuse to return a log");
    let mounts = fs::read_to_string("/proc/self/mounts").unwrap();
    assert!(!mounts.contains(mnt.to_str().unwrap()), "unmounted");
    (err, unmounted)
}

#[test]
fn a_classic_handler_panic_aborts_the_step_and_fails_the_run() {
    if !enabled() || !in_child("a_classic_handler_panic_aborts_the_step_and_fails_the_run") {
        return;
    }
    let (err, unmounted) = fault_on(Transport::Classic);
    assert_eq!(err.raw_os_error(), Some(libc_eio()), "EIO expected: {err}");
    match unmounted {
        FsError::Fatal { what } => assert!(
            what.contains("lookup panicked") && what.contains("injected fault"),
            "{what}"
        ),
        other => panic!("expected a fatal observer error, got {other:?}"),
    }
}

#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y)"]
fn a_ring_queue_panic_aborts_the_step_and_fails_the_run() {
    if !enabled() || !in_child("a_ring_queue_panic_aborts_the_step_and_fails_the_run") {
        return;
    }
    let (_err, unmounted) = fault_on(Transport::Uring);
    match unmounted {
        FsError::Io { what, .. } => assert!(
            what.contains("ring queue") && what.contains("injected fault"),
            "{what}"
        ),
        other => panic!("expected the ring's I/O error, got {other:?}"),
    }
}

fn libc_eio() -> i32 {
    5
}
