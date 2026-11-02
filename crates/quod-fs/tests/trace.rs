//! The observer's trace for test-level attribution (docs/test-level.md):
//! through the kernel overlay a step really sees, every process's lookup,
//! stat and open of a file reaches the observer again after a marker, so
//! an access lands in the window it happened in, not only the first one.
//! Needs Linux and /dev/fuse; gated on QUOD_INTEGRATION=1.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::Command;

use quod_cas::{Cas, Kind};
use quod_fs::{Options, Trace, TraceEvent, Traced, Transport, marker_name};

fn enabled() -> bool {
    let on = std::env::var("QUOD_INTEGRATION").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("QUOD_INTEGRATION != 1; skipping");
    }
    on
}

const TOKEN: &str = "t0k3n";

fn sh(cwd: &Path, script: &str) {
    let out = Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{script}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Marker `n`, as a separate process looks it up (its own stat).
fn mark(n: u64) -> String {
    format!("stat {} >/dev/null 2>&1 || true", marker_name(TOKEN, n))
}

/// The accesses of `path` between markers `a` and `b` (by sequence).
fn between<'t>(t: &'t Trace, a: u64, b: u64, path: &str) -> Vec<&'t TraceEvent> {
    let seq = |n: u64| {
        t.events
            .iter()
            .find(|e| e.what == Traced::Marker(n))
            .unwrap_or_else(|| panic!("marker {n} not traced"))
            .seq
    };
    let (sa, sb) = (seq(a), seq(b));
    t.events
        .iter()
        .filter(|e| e.seq > sa && e.seq < sb)
        .filter(|e| matches!(&e.what, Traced::Access(x) if x.path == path))
        .collect()
}

fn traced(transport: Transport) {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join("pkg")).unwrap();
    fs::write(src.join("pkg/data.txt"), "hello\n").unwrap();
    fs::write(src.join("pkg/other.txt"), "other\n").unwrap();
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas.import_dir(&src).unwrap();
    let lower = tmp.path().join("lower");
    fs::create_dir_all(&lower).unwrap();
    let opts = Options {
        transport,
        trace: Some(TOKEN.into()),
        // The caller's default for every non-Go step.
        zero_open: true,
        ..Options::default()
    };
    let served = quod_fs::serve_with(&tree, &cas, &lower, &opts).unwrap();
    let ov = quod_fs::overlay(
        &served,
        &tmp.path().join("upper"),
        &tmp.path().join("work"),
        &tmp.path().join("merged"),
    )
    .unwrap();
    let m = ov.merged().to_path_buf();
    // Window 1: one process reads the file. Window 2: another process only
    // stats it. Window 3: a third opens it. Window 4: a fourth looks up an
    // absent name twice (the second from the overlay's negative cache).
    sh(&m, &mark(1));
    sh(&m, "cat pkg/data.txt >/dev/null");
    sh(&m, &mark(2));
    sh(&m, "stat pkg/data.txt >/dev/null");
    sh(&m, &mark(3));
    sh(&m, "head -c1 pkg/data.txt >/dev/null");
    sh(&m, &mark(4));
    sh(&m, "test -e pkg/nope || true; test -e pkg/nope || true");
    sh(&m, &mark(5));
    sh(&m, "test -e pkg/nope || true; cat pkg/data.txt >/dev/null");
    sh(&m, &mark(6));
    drop(ov);
    let observed = served.unmount_observed().unwrap();
    let t = observed.trace.expect("a traced mount returns its trace");
    // Markers are never accesses.
    assert!(
        observed
            .accesses
            .iter()
            .all(|a| !a.path.contains(quod_fs::MARKER_PREFIX)),
        "a marker was logged as an access"
    );
    for (a, b, what) in [(1, 2, "cat"), (2, 3, "stat"), (3, 4, "open"), (5, 6, "cat again")] {
        let seen = between(&t, a, b, "pkg/data.txt");
        assert!(
            seen.iter()
                .any(|e| matches!(&e.what, Traced::Access(x) if x.kind == Kind::File)),
            "window {a}-{b} ({what}): pkg/data.txt not traced; trace: {:#?}",
            t.events
        );
        // Learned from /proc while the request was in flight.
        for e in seen {
            assert!(t.procs.contains_key(&e.tid), "no process info for {}", e.tid);
        }
    }
    assert!(
        !between(&t, 4, 5, "pkg/nope").is_empty(),
        "the first absent lookup reaches the observer"
    );
    // Sequence numbers are strictly increasing in log order.
    assert!(t.events.windows(2).all(|w| w[0].seq < w[1].seq));
    // The second absent lookup (window 5) may come from the overlay's
    // negative dentry: test-level attribution never relies on seeing it
    // (absent entries are the session's, docs/test-level.md).
    eprintln!(
        "absent repeat seen in window 5: {}",
        !between(&t, 5, 6, "pkg/nope").is_empty()
    );
}

#[test]
fn every_window_sees_its_file_accesses_classic() {
    if !enabled() {
        return;
    }
    traced(Transport::Classic);
}

#[test]
#[ignore = "needs FUSE over io_uring (the merge gate's ring suites)"]
fn every_window_sees_its_file_accesses_uring() {
    if !enabled() {
        return;
    }
    traced(Transport::Uring);
}
