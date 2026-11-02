//! A `quod run` stopped by a signal cleans up after itself, and what a
//! SIGKILL leaves is swept by the next run, end to end through the binary.
//! Gated on QUOD_INTEGRATION=1 like the rest.

mod common;
use common::*;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Mount points under `runs`, from /proc/self/mountinfo (field 5; the
/// paths here need no unescaping).
fn mounts_under(runs: &Path) -> Vec<PathBuf> {
    let runs = runs.to_str().unwrap();
    std::fs::read_to_string("/proc/self/mountinfo")
        .unwrap()
        .lines()
        .filter_map(|l| l.split(' ').nth(4))
        .filter(|p| p.starts_with(runs))
        .map(PathBuf::from)
        .collect()
}

/// Run directories (not lock files) under `runs`.
fn run_dirs(runs: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(runs)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

fn spawn(env: &Env, state: &Path, extra: &[&str], stderr: Stdio) -> Child {
    seed_rootfs_in(state);
    Command::new(env!("CARGO_BIN_EXE_quod"))
        .arg("--state")
        .arg(state)
        .args([
            "run",
            "--repo",
            env.repo.to_str().unwrap(),
            "--toolchain",
            "go",
            "--step",
            "sleep 60",
        ])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(stderr)
        .spawn()
        .unwrap()
}

/// Polls `until` every 10 ms for 30 s.
fn wait_for(what: &str, mut until: impl FnMut() -> bool) {
    let t = Instant::now();
    while !until() {
        assert!(t.elapsed() < Duration::from_secs(30), "never: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_exit(child: &mut Child) -> std::process::ExitStatus {
    let t = Instant::now();
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            return st;
        }
        if t.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("quod did not exit after the signal");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn signal(child: &Child, sig: i32) {
    // SAFETY: kill(2) on our own child's pid.
    #[allow(unsafe_code)]
    let rc = unsafe { libc::kill(child.id() as i32, sig) };
    assert_eq!(rc, 0, "kill: {}", std::io::Error::last_os_error());
}

/// SIGTERM mid-step: the step is stopped, every mount of the run is gone,
/// so is its directory, and quod exits 128+15.
#[test]
fn sigterm_mid_step_unmounts_and_exits_143() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let state = env.tmp.path().join("state-term");
    std::fs::create_dir_all(state.join("runs")).unwrap();
    let runs = state.join("runs").canonicalize().unwrap();
    let mut child = spawn(&env, &state, &[], Stdio::null());
    wait_for("the run mounted its workspace", || {
        !mounts_under(&runs).is_empty()
    });
    signal(&child, libc::SIGTERM);
    let st = wait_exit(&mut child);
    assert_eq!(st.code(), Some(143), "{st}");
    assert!(mounts_under(&runs).is_empty(), "{:?}", mounts_under(&runs));
    assert!(run_dirs(&runs).is_empty(), "{:?}", run_dirs(&runs));
}

/// SIGHUP with the terminal gone (here: stderr's reader closed, so every
/// write to it fails, as a hung-up pty's do with EIO): quod still tears
/// down and exits 128+1, never panicking on its own report.
#[test]
fn sighup_with_stderr_gone_still_cleans_up_without_panicking() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let state = env.tmp.path().join("state-hup");
    std::fs::create_dir_all(state.join("runs")).unwrap();
    let runs = state.join("runs").canonicalize().unwrap();
    let mut child = spawn(&env, &state, &[], Stdio::piped());
    wait_for("the run mounted its workspace", || {
        !mounts_under(&runs).is_empty()
    });
    drop(child.stderr.take());
    signal(&child, libc::SIGHUP);
    let st = wait_exit(&mut child);
    assert_eq!(st.code(), Some(129), "{st}");
    assert!(mounts_under(&runs).is_empty(), "{:?}", mounts_under(&runs));
    assert!(run_dirs(&runs).is_empty(), "{:?}", run_dirs(&runs));
}

/// A network step claims its run directory like an observed step: locked
/// while it runs, so what a SIGKILL leaves is swept by the next run.
#[test]
fn a_killed_network_steps_run_dir_is_swept() {
    if !enabled() {
        return;
    }
    let env = Env::new();
    let state = env.tmp.path().join("state-net");
    std::fs::create_dir_all(state.join("runs")).unwrap();
    let runs = state.join("runs").canonicalize().unwrap();
    let mut child = spawn(&env, &state, &["--allow-net"], Stdio::null());
    let locks = || -> Vec<PathBuf> {
        std::fs::read_dir(&runs)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "lock"))
            .collect()
    };
    wait_for("the network run claimed its directory", || {
        !locks().is_empty() && !run_dirs(&runs).is_empty()
    });
    let dead = run_dirs(&runs);
    child.kill().unwrap();
    child.wait().unwrap();
    // The step (bwrap dies with its parent) is gone; its directory stays
    // until the next run claims one.
    let next = env.quod_in(
        &state,
        &[
            "run",
            "--repo",
            env.repo.to_str().unwrap(),
            "--toolchain",
            "go",
            "--allow-net",
            "--step",
            "true",
        ],
        &[],
    );
    assert_eq!(next.code, 0, "{}", next.stderr);
    for d in &dead {
        assert!(!d.exists(), "{} was not swept", d.display());
    }
    assert!(locks().is_empty(), "{:?}", locks());
}
