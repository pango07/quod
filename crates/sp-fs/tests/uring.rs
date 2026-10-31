//! The ring transport's failure paths: a ring that cannot come up falls
//! back to the classic transport before anything is served, and a queue
//! thread that dies while serving fails the step at once and makes the
//! unmount an error, never a hang and never a log.
//!
//! Each test needs FUSE over io_uring (Linux, root, `fuse.enable_uring=Y`,
//! a kernel whose FUSE minor the ring transport is tested against), so each
//! is #[ignore]d: `cargo test -p sp-fs --test uring -- --ignored`. Run on a
//! host without the ring, each fails saying so; none passes vacuously.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sp_cas::{Cas, Tree};
use sp_fs::{Fault, Options, Served, Transport};

fn sample(dir: &Path) {
    fs::create_dir_all(dir.join("pkg/cmd")).unwrap();
    fs::write(dir.join("go.mod"), "module x\n").unwrap();
    fs::write(dir.join("pkg/cmd/root.go"), "package cmd\n").unwrap();
}

struct Fixture {
    tmp: tempfile::TempDir,
    cas: Cas,
    tree: Tree,
}

fn fixture() -> Fixture {
    assert!(
        std::env::var("STILLPASS_INTEGRATION").is_ok_and(|v| v == "1"),
        "these tests mount FUSE; set STILLPASS_INTEGRATION=1 (scripts/dev.sh does)"
    );
    let tmp = tempfile::tempdir().unwrap();
    sample(&tmp.path().join("src"));
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas.import_dir(&tmp.path().join("src")).unwrap();
    Fixture { tmp, cas, tree }
}

impl Fixture {
    fn serve(
        &self,
        name: &str,
        transport: Transport,
        fault: Option<Fault>,
    ) -> Result<Served, String> {
        let mnt = self.tmp.path().join(name);
        fs::create_dir_all(&mnt).unwrap();
        let opts = Options {
            transport,
            fault,
            ..Options::default()
        };
        sp_fs::serve_with(&self.tree, &self.cas, &mnt, &opts).map_err(|e| e.to_string())
    }

    /// A healthy ring mount first, so a host without the ring fails here
    /// with a reason instead of passing a fallback test for the wrong one.
    fn require_ring(&self) {
        let s = self
            .serve("ring-check", Transport::Uring, None)
            .unwrap_or_else(|e| {
                panic!("FUSE over io_uring is unavailable here, so this test cannot run: {e}")
            });
        assert!(s.stats().uring);
        s.unmount().unwrap();
    }
}

/// Runs `script` in `dir`, allowed `limit`. Past it the mount is aborted
/// (`umount -f`, which ends every waiting request) and the test fails: the
/// failure under test turned into a hang.
fn bounded(dir: &Path, script: &str, limit: Duration) -> (bool, Duration) {
    let t = Instant::now();
    // The shell enters the mount itself: that cd is already a request, and
    // its failure is the step's, not the test harness's.
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(format!("cd {} && {script}", dir.display()))
        .current_dir("/")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            return (st.success(), t.elapsed());
        }
        if t.elapsed() > limit {
            let _ = Command::new("umount").arg("-f").arg(dir).status();
            let _ = child.wait();
            panic!("{script:?} still blocked after {limit:?}: a dead ring thread hung the step");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn dies_mid_mount(fault: Fault, expect: &str) {
    let f = fixture();
    f.require_ring();
    let served = f
        .serve("m", Transport::Uring, Some(fault))
        .expect("the fault fires only once serving");
    assert!(served.stats().uring);
    let mnt = served.mountpoint().to_path_buf();
    // The first ring request kills its queue thread; the abort that follows
    // fails it and everything after it.
    let (ok, took) = bounded(
        &mnt,
        "cat go.mod pkg/cmd/root.go && ls pkg && stat go.mod",
        Duration::from_secs(60),
    );
    assert!(
        !ok,
        "the step saw a working filesystem after its observer died"
    );
    assert!(took < Duration::from_secs(30), "took {took:?}");
    let t = Instant::now();
    let err = served
        .unmount()
        .expect_err("a dead queue thread must make the unmount an error")
        .to_string();
    assert!(
        t.elapsed() < Duration::from_secs(30),
        "unmount took {:?}",
        t.elapsed()
    );
    assert!(err.contains(expect), "{err}");
    let mounts = fs::read_to_string("/proc/self/mounts").unwrap();
    assert!(!mounts.contains(mnt.to_str().unwrap()), "left mounted");
}

#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y); run with --ignored"]
fn a_queue_thread_that_panics_fails_the_step_and_the_unmount() {
    dies_mid_mount(Fault::QueuePanic(0), "panicked");
}

#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y); run with --ignored"]
fn a_failed_completion_fails_the_step_and_the_unmount() {
    dies_mid_mount(Fault::QueueError(0), "completion of entry");
}

fn falls_back(fault: Fault, why: &str) {
    let f = fixture();
    f.require_ring();
    // Forced, the ring's failure is the mount's.
    let err = match f.serve("forced", Transport::Uring, Some(fault)) {
        Ok(_) => panic!("{fault:?}: a forced ring mount came up"),
        Err(e) => e,
    };
    assert!(err.contains(why), "{err}");
    let mounts = fs::read_to_string("/proc/self/mounts").unwrap();
    assert!(
        !mounts.contains("/forced"),
        "a failed ring mount stayed mounted"
    );
    // Auto: the classic transport serves the whole mount, and says so.
    let served = f.serve("auto", Transport::Auto, Some(fault)).unwrap();
    let s = served.stats();
    assert!(!s.uring && s.uring_fallback, "{s:?}");
    assert_eq!((s.ring_requests, s.device_requests), (0, 0), "{s:?}");
    let (ok, _) = bounded(
        served.mountpoint(),
        "cat go.mod >/dev/null",
        Duration::from_secs(60),
    );
    assert!(ok);
    let log = served.unmount().unwrap();
    let classic = f.serve("classic", Transport::Classic, None).unwrap();
    let (ok, _) = bounded(
        classic.mountpoint(),
        "cat go.mod >/dev/null",
        Duration::from_secs(60),
    );
    assert!(ok);
    assert_eq!(
        log,
        classic.unmount().unwrap(),
        "the fallback logs as classic does"
    );
}

#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y); run with --ignored"]
fn a_refused_registration_falls_back_before_anything_is_served() {
    falls_back(Fault::Register, "REGISTER");
}

#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y); run with --ignored"]
fn a_ring_that_never_becomes_ready_falls_back() {
    falls_back(Fault::Unready, "never became ready");
}
