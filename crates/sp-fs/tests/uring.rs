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

/// The mounts at exactly `path` in this process's mount namespace, as
/// device numbers (major, minor), from /proc/self/mountinfo field 5. Only
/// this test's own paths are asked about, so a mount another test (running
/// in parallel) holds at a path that merely contains the same name is never
/// counted.
fn mounts_at(path: &Path) -> Vec<(u32, u32)> {
    let want = path.to_str().unwrap();
    assert!(
        !want.contains([' ', '\t', '\n', '\\']),
        "mountinfo escapes {want:?}"
    );
    fs::read_to_string("/proc/self/mountinfo")
        .unwrap()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            (f.get(4) == Some(&want)).then(|| {
                let (a, b) = f[2].split_once(':').unwrap();
                (a.parse().unwrap(), b.parse().unwrap())
            })
        })
        .collect()
}

/// The FUSE connection id of the topmost mount at `path` (its directory
/// name under fusectl: the superblock's dev_t, MKDEV(major, minor)).
fn conn_at(path: &Path) -> u64 {
    let (major, minor) = *mounts_at(path).last().expect("not a mount point");
    (u64::from(major) << 20) | u64::from(minor)
}

/// One test at a time in this binary: each mounts and unmounts FUSE
/// connections, and the kernel hands a freed connection id to the next
/// mount, so a parallel test's new connection would look like a dead one
/// that outlived its mount. (A freed id can still be reused by a mount
/// elsewhere on the machine; `FuseCtl::lists_since` tells those apart.)
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A private mount of fusectl, which lists every FUSE connection the kernel
/// still has (one directory per connection, gone when it is destroyed).
struct FuseCtl(tempfile::TempDir);

impl FuseCtl {
    fn mount() -> FuseCtl {
        let dir = tempfile::tempdir().unwrap();
        let st = Command::new("mount")
            .args(["-t", "fusectl", "none"])
            .arg(dir.path())
            .status()
            .unwrap();
        assert!(st.success(), "mount fusectl (needs root)");
        FuseCtl(dir)
    }

    fn lists(&self, conn: u64) -> bool {
        self.0.path().join(conn.to_string()).exists()
    }

    /// The inode of connection `conn`'s directory: fusectl gives every new
    /// connection a new one, so (id, inode) names one connection even when
    /// a later mount reuses the id.
    fn ino(&self, conn: u64) -> Option<u64> {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(self.0.path().join(conn.to_string()))
            .ok()
            .map(|m| m.ino())
    }

    /// Whether fusectl lists a connection `conn` created before `t`:
    /// fusectl stamps a connection's directory with the time it was
    /// created, so one created after `t` is a reuse of the id by a newer
    /// mount, not the connection that had it before.
    fn lists_since(&self, conn: u64, t: std::time::SystemTime) -> bool {
        match fs::metadata(self.0.path().join(conn.to_string())) {
            Ok(m) => m.modified().map_or(true, |c| c <= t),
            Err(_) => false,
        }
    }
}

impl Drop for FuseCtl {
    fn drop(&mut self) {
        let _ = Command::new("umount").arg(self.0.path()).status();
    }
}

/// The connection id a failed ring mount's error names.
fn named_conn(err: &str) -> u64 {
    let at = err
        .find("FUSE connection ")
        .unwrap_or_else(|| panic!("no connection named in {err:?}"))
        + "FUSE connection ".len();
    err[at..]
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

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

/// How long a serve or an unmount may take before the test calls it a hang
/// (a failed setup waits up to 10 s for its readiness probe).
const LIMIT: Duration = Duration::from_secs(60);

/// Runs `f` on its own thread and fails the test if it takes past `LIMIT`:
/// a hang is a failure, not a stuck test run.
fn within<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(LIMIT)
        .unwrap_or_else(|_| panic!("{what} still running after {LIMIT:?}: a hang"))
}

fn unmount(s: Served) -> Result<Vec<sp_manifest::Access>, String> {
    within("unmount", move || s.unmount().map_err(|e| e.to_string()))
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
        let (tree, cas) = (self.tree.clone(), self.cas.clone());
        within(&format!("serving {name}"), move || {
            sp_fs::serve_with(&tree, &cas, &mnt, &opts).map_err(|e| e.to_string())
        })
    }

    /// A healthy ring mount first, so a host without the ring fails here
    /// with a reason instead of passing a fallback test for the wrong one.
    /// Also the control for `FuseCtl`: it lists this mount's connection
    /// while mounted and not after.
    fn require_ring(&self, ctl: &FuseCtl) {
        let s = self
            .serve("ring-check", Transport::Uring, None)
            .unwrap_or_else(|e| {
                panic!("FUSE over io_uring is unavailable here, so this test cannot run: {e}")
            });
        assert!(s.stats().uring);
        let mnt = s.mountpoint().to_path_buf();
        let conn = conn_at(&mnt);
        assert!(ctl.lists(conn), "fusectl does not list a live connection");
        unmount(s).unwrap();
        let t = std::time::SystemTime::now();
        assert!(mounts_at(&mnt).is_empty() && !ctl.lists_since(conn, t));
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
    let _one = serial();
    let f = fixture();
    let ctl = FuseCtl::mount();
    f.require_ring(&ctl);
    let served = f
        .serve("m", Transport::Uring, Some(fault))
        .expect("the fault fires only once serving");
    assert!(served.stats().uring);
    let mnt = served.mountpoint().to_path_buf();
    let conn = conn_at(&mnt);
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
    let err = unmount(served).expect_err("a dead queue thread must make the unmount an error");
    assert!(
        t.elapsed() < Duration::from_secs(30),
        "unmount took {:?}",
        t.elapsed()
    );
    assert!(err.contains(expect), "{err}");
    let t = std::time::SystemTime::now();
    assert!(mounts_at(&mnt).is_empty(), "left mounted");
    assert!(
        !ctl.lists_since(conn, t),
        "the dead ring's connection outlived the unmount"
    );
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
    let _one = serial();
    let f = fixture();
    let ctl = FuseCtl::mount();
    f.require_ring(&ctl);
    // Forced, the ring's failure is the mount's, and when serve returns the
    // ring mount is gone from this test's own path and its connection from
    // the kernel.
    let err = match f.serve("forced", Transport::Uring, Some(fault)) {
        Ok(_) => panic!("{fault:?}: a forced ring mount came up"),
        Err(e) => e,
    };
    let returned = std::time::SystemTime::now();
    assert!(err.contains(why), "{err}");
    // serve checked its own teardown: unmounted outright, not detached.
    assert!(!err.contains("not torn down"), "{err}");
    let forced = f.tmp.path().join("forced");
    assert!(
        mounts_at(&forced).is_empty(),
        "a failed ring mount stayed mounted at {}",
        forced.display()
    );
    let conn = named_conn(&err);
    assert!(
        !ctl.lists_since(conn, returned),
        "connection {conn} outlived the failed mount"
    );
    // Auto: the classic transport serves the whole mount, and says so. The
    // ring mount it abandoned was torn down first: the path holds exactly
    // one mount (classic's, not stacked on the ring's), and the abandoned
    // connection is gone while classic's is live.
    let served = f.serve("auto", Transport::Auto, Some(fault)).unwrap();
    let returned = std::time::SystemTime::now();
    let s = served.stats();
    assert!(!s.uring && s.uring_fallback, "{s:?}");
    assert_eq!((s.ring_requests, s.device_requests), (0, 0), "{s:?}");
    let auto = served.mountpoint().to_path_buf();
    assert_eq!(
        mounts_at(&auto).len(),
        1,
        "the fallback stacked on the ring mount"
    );
    let ring = s
        .ring_abandoned_conn
        .expect("the abandoned ring connection");
    let classic_conn = conn_at(&auto);
    assert!(ctl.lists(classic_conn), "classic's connection is not live");
    if classic_conn == ring {
        // The kernel gave the abandoned connection's id to the classic
        // mount. A FUSE mount's id is its superblock's anonymous device
        // number, freed only when that superblock is killed, which is
        // where the kernel destroys the connection (fuse_kill_sb_anon):
        // the ring's connection was gone before classic mounted.
    } else {
        assert!(
            !ctl.lists_since(ring, returned),
            "the abandoned ring connection {ring} is alive"
        );
    }
    let (ok, _) = bounded(
        served.mountpoint(),
        "cat go.mod >/dev/null",
        Duration::from_secs(60),
    );
    assert!(ok);
    let log = unmount(served).unwrap();
    let classic = f.serve("classic", Transport::Classic, None).unwrap();
    let (ok, _) = bounded(
        classic.mountpoint(),
        "cat go.mod >/dev/null",
        Duration::from_secs(60),
    );
    assert!(ok);
    assert_eq!(
        log,
        unmount(classic).unwrap(),
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

/// Holds the first mount that appears at `path` busy from another process:
/// an O_PATH descriptor of its root (which sends FUSE nothing, so it cannot
/// block on a ring that is not ready) given to a `sleep` as its stdin.
/// Returns the holder and the held mount's (connection id, fusectl inode).
fn hold_when_mounted(path: &Path, ctl: &FuseCtl) -> (std::process::Child, u64, u64) {
    use std::os::unix::fs::OpenOptionsExt;
    let deadline = Instant::now() + Duration::from_secs(9);
    while mounts_at(path).is_empty() {
        assert!(
            Instant::now() < deadline,
            "nothing was mounted at {}",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let root = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH)
        .open(path)
        .unwrap();
    let conn = conn_at(path);
    let ino = ctl.ino(conn).expect("fusectl lists the held connection");
    let child = Command::new("sleep")
        .arg("120")
        .stdin(Stdio::from(root))
        .spawn()
        .unwrap();
    (child, conn, ino)
}

/// A ring mount that something outside still holds when its setup fails
/// cannot be unmounted, only detached, and its connection lives on (aborted)
/// until the holder lets go. Falling back to classic then would put a new
/// mount over a ring connection that still exists, so serve must refuse:
/// a Stillpass error, nothing mounted at the path. This is checked from the
/// outside (serve's result, mountinfo, the test's own fusectl), not by the
/// code's wording; the teardown before this fix (abort, detach at once)
/// reported such a mount as torn down and fell back.
#[test]
#[ignore = "needs FUSE over io_uring (root, fuse.enable_uring=Y); run with --ignored"]
fn a_ring_mount_held_through_its_teardown_is_not_replaced() {
    let _one = serial();
    let f = fixture();
    let ctl = FuseCtl::mount();
    f.require_ring(&ctl);
    let held = f.tmp.path().join("held");
    fs::create_dir_all(&held).unwrap();
    // The `unready` hook keeps the failing setup waiting 10 s for its
    // probe: time to take hold of the mount before the teardown starts.
    let (tree, cas, at) = (f.tree.clone(), f.cas.clone(), held.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let opts = Options {
            transport: Transport::Auto,
            fault: Some(Fault::Unready),
            ..Options::default()
        };
        let _ = tx.send(sp_fs::serve_with(&tree, &cas, &at, &opts).map(|s| s.stats()));
    });
    let (mut holder, conn, ino) = hold_when_mounted(&held, &ctl);
    let served = rx.recv_timeout(LIMIT).expect("serve hung");
    let alive = ctl.ino(conn) == Some(ino);
    let mounted = mounts_at(&held);
    let _ = holder.kill();
    let _ = holder.wait();
    let err = match served {
        Ok(stats) => panic!(
            "serve fell back to classic ({stats:?}) while ring connection {conn} was still alive: {alive}"
        ),
        Err(e) => e.to_string(),
    };
    assert!(
        alive,
        "the held connection should still exist (aborted) while held"
    );
    assert!(
        mounted.is_empty(),
        "something is mounted at the path: {mounted:?}"
    );
    eprintln!("serve refused: {err}");
    // Once released, the detached mount and its connection go.
    let deadline = Instant::now() + Duration::from_secs(10);
    while ctl.ino(conn) == Some(ino) {
        assert!(
            Instant::now() < deadline,
            "connection {conn} outlived its holder"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
