//! The ring transport's failure paths through the CLI, with the
//! test-only failure hooks (`QUOD_TEST_URING_FAULT`, compiled in only
//! with the `fault-injection` feature): a ring that cannot come up is a
//! recorded fallback to classic, a ring that dies under the step is a fast
//! Quod error with nothing recorded.
//!
//! Needs the ring (Linux, root, `fuse.enable_uring=Y`, FUSE 7.42-7.45), so
//! every test is #[ignore]d:
//! `cargo test -p quod-cli --features fault-injection --test ring_faults -- --ignored`.
//! On a host without the ring each fails saying so.
mod common;

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use common::*;
use quod_ledger::{Ledger, RowCounts};

const STEP: &str = "cat go.mod && go test ./...";

fn ring_needed(e: impl std::fmt::Display) -> ! {
    panic!("FUSE over io_uring is unavailable here ({e}), so this test cannot run")
}

/// Runs the CLI with a ring failure hook, bounded: a run still going after
/// 300s fails the test (a hang). Returns the output and how long it took.
fn hooked(env: &Env, state: &Path, fault: &str, transport: Option<&str>) -> (Out, Duration) {
    let mut envs = vec![("QUOD_TEST_URING_FAULT".to_string(), fault.to_string())];
    if let Some(t) = transport {
        envs.push(("QUOD_FUSE_TRANSPORT".into(), t.into()));
    }
    // Rootfs mode (QUOD_ROOTFS set): this state dir needs the rootfs too.
    seed_rootfs_in(state);
    let (tx, rx) = std::sync::mpsc::channel();
    let (repo, st) = (env.repo.to_str().unwrap().to_string(), state.to_path_buf());
    let t = Instant::now();
    std::thread::spawn(move || {
        let out = Command::new(env!("CARGO_BIN_EXE_quod"))
            .arg("--state")
            .arg(&st)
            .args(["run", "--repo", &repo, "--step", STEP])
            .envs(envs)
            .output()
            .unwrap();
        let _ = tx.send(out);
    });
    let out = rx
        .recv_timeout(Duration::from_secs(300))
        .unwrap_or_else(|_| panic!("QUOD_TEST_URING_FAULT={fault}: the run hung"));
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

/// The errors the observer recorded for the step (its `I/O error served to
/// the step` events).
fn observer_errors(state: &Path) -> Vec<String> {
    events(state)
        .into_iter()
        .filter(|e| e["message"] == "I/O error served to the step")
        .map(|e| e["error"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y); run with --ignored"]
fn a_ring_queue_that_dies_under_the_step_fails_the_run_fast() {
    assert!(enabled(), "needs Linux and QUOD_INTEGRATION=1");
    let env = Env::new();
    // The baseline: the same step on a healthy ring passes and is recorded,
    // its queue threads at the default nice value.
    let ok_state = env.tmp.path().join("state-ok");
    let o = env.quod_in(
        &ok_state,
        &["run", "--repo", env.repo.to_str().unwrap(), "--step", STEP],
        &[("QUOD_FUSE_TRANSPORT", "uring")],
    );
    if o.code == 2 {
        ring_needed(&o.stderr);
    }
    assert!(o.executed() && o.code == 0, "{}", o.stderr);
    let run = last_event(&ok_state, "run").unwrap();
    assert!(
        run["observer"]
            .as_str()
            .unwrap()
            .starts_with("fuse-uring+nice-10+"),
        "{run}"
    );
    let obs = last_event(&ok_state, "observer").unwrap();
    assert_eq!(
        (obs["ring_nice"].as_i64(), obs["ring_nice_denied"].as_bool()),
        (Some(-10), Some(false)),
        "{obs}"
    );
    // Half the baseline's ring requests in, the failure lands inside go
    // test; with no count it lands on the first request after the mount
    // came up (the overlay mounting over it).
    let half = obs["ring_requests"].as_u64().unwrap() / 2;
    assert!(half > 10, "the step sends too few requests to aim at");
    let mid = [format!("panic:{half}"), format!("error:{half}")];
    for fault in ["panic", "error", &mid[0], &mid[1]] {
        let state = env
            .tmp
            .path()
            .join(format!("state-{}", fault.replace(':', "-")));
        let (o, took) = hooked(&env, &state, fault, Some("uring"));
        assert_eq!(
            o.code, 2,
            "{fault}: a dead observer must be a Quod error: {}",
            o.stderr
        );
        assert!(o.stderr.contains("QUOD  ERROR"), "{}", o.stderr);
        assert!(!o.executed(), "{fault}: reported as a step result");
        assert!(took < Duration::from_secs(120), "{fault}: took {took:?}");
        // The failure is the ring's: the observer recorded the dead queue
        // thread, by the cause the hook injected.
        let cause = if fault.starts_with("panic") {
            "panicked: test hook: ring queue"
        } else {
            "failed: completion of entry"
        };
        let errors = observer_errors(&state);
        assert!(
            errors
                .iter()
                .any(|e| e.starts_with("FUSE over io_uring: ring queue") && e.contains(cause)),
            "{fault}: no ring failure among the observer's errors {errors:?}"
        );
        if fault.contains(':') {
            // The step ran and failed; the observer's record of why is the
            // error the run reports.
            assert!(
                o.stderr.contains("FUSE over io_uring: ring queue"),
                "{}",
                o.stderr
            );
        }
        // Nothing recorded, counted row by row.
        let ledger = Ledger::open(&state.join("ledger.sqlite")).unwrap();
        assert_eq!(
            ledger.row_counts().unwrap(),
            RowCounts {
                runs: 0,
                manifests: 0,
                results: 0
            },
            "{fault}: something was recorded"
        );
    }
}

#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y); run with --ignored"]
fn a_ring_that_cannot_come_up_is_a_recorded_fallback() {
    assert!(enabled(), "needs Linux and QUOD_INTEGRATION=1");
    let env = Env::new();
    // Forced, the ring's failure is the run's, and nothing is recorded.
    let state = env.tmp.path().join("state-forced");
    let (o, _) = hooked(&env, &state, "register", Some("uring"));
    assert_eq!(o.code, 2, "{}", o.stderr);
    if !o.stderr.contains("REGISTER") {
        ring_needed(&o.stderr);
    }
    assert_eq!(
        Ledger::open(&state.join("ledger.sqlite"))
            .unwrap()
            .row_counts()
            .unwrap()
            .results,
        0
    );
    // Auto: classic serves the whole mount, and the run says so.
    let state = env.tmp.path().join("state-auto");
    let (o, _) = hooked(&env, &state, "register", None);
    assert!(o.executed() && o.code == 0, "{}", o.stderr);
    let run = last_event(&state, "run").unwrap();
    assert_eq!(run["transport"], "fuse", "{run}");
    let observer = run["observer"].as_str().unwrap();
    assert!(observer.starts_with("fuse+uring-fallback+"), "{run}");
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

/// The first mount whose mount point is `<runs>/<run id>/lower`, from
/// /proc/self/mountinfo (field 5; the paths here need no unescaping).
fn run_lower_mount(runs: &Path) -> Option<std::path::PathBuf> {
    let runs = runs.to_str().unwrap();
    std::fs::read_to_string("/proc/self/mountinfo")
        .ok()?
        .lines()
        .filter_map(|l| l.split(' ').nth(4))
        .find(|p| p.starts_with(runs) && p.ends_with("/lower"))
        .map(std::path::PathBuf::from)
}

/// A ring mount that something outside holds through its failed setup's
/// teardown can only be detached, its connection alive (aborted) until the
/// holder lets go, so the run must not fall back to classic: it is a
/// Quod error with nothing recorded.
#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y); run with --ignored"]
fn a_ring_mount_held_through_its_teardown_fails_the_run() {
    use std::os::unix::fs::OpenOptionsExt;
    assert!(enabled(), "needs Linux and QUOD_INTEGRATION=1");
    let env = Env::new();
    let state = env.tmp.path().join("state-held");
    // Rootfs mode (QUOD_ROOTFS set): this state dir needs the rootfs too.
    seed_rootfs_in(&state);
    let t = Instant::now();
    // `unready`: the setup waits 10 s for its readiness probe, time to take
    // hold of the mount before the teardown starts.
    let mut run = Command::new(env!("CARGO_BIN_EXE_quod"))
        .arg("--state")
        .arg(&state)
        .args(["run", "--repo", env.repo.to_str().unwrap(), "--step", STEP])
        .env("QUOD_TEST_URING_FAULT", "unready")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let runs = state.join("runs");
    let lower = loop {
        if let Some(p) = run_lower_mount(&runs) {
            break p;
        }
        assert!(
            t.elapsed() < Duration::from_secs(9),
            "the run never mounted its ring"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    // An O_PATH descriptor of the mount's root sends FUSE nothing (so it
    // cannot block on the unready ring) and keeps the mount busy.
    let root = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH)
        .open(&lower)
        .unwrap();
    let mut holder = Command::new("sleep")
        .arg("120")
        .stdin(std::process::Stdio::from(root))
        .spawn()
        .unwrap();
    let status = loop {
        if let Some(st) = run.try_wait().unwrap() {
            break st;
        }
        if t.elapsed() > Duration::from_secs(120) {
            let _ = run.kill();
            let _ = holder.kill();
            panic!("the run hung with its ring mount held");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = holder.kill();
    let _ = holder.wait();
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut run.stderr.take().unwrap(), &mut stderr).unwrap();
    eprintln!("held ring mount: {status} in {:?}\n{stderr}", t.elapsed());
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("QUOD  ERROR"), "{stderr}");
    assert!(
        !stderr.contains("QUOD  EXECUTED"),
        "fell back and ran: {stderr}"
    );
    assert_eq!(
        Ledger::open(&state.join("ledger.sqlite"))
            .unwrap()
            .row_counts()
            .unwrap(),
        RowCounts {
            runs: 0,
            manifests: 0,
            results: 0
        }
    );
}
