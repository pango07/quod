//! The held-mount test alone, to run against main's teardown (before fix/uring-fallback).
#![cfg(target_os = "linux")]
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use sp_cas::{Cas, Tree};
use sp_fs::{Fault, Options, Transport};
const LIMIT: Duration = Duration::from_secs(60);

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


struct FuseCtl(tempfile::TempDir);
impl FuseCtl {
    fn mount() -> FuseCtl {
        let dir = tempfile::tempdir().unwrap();
        assert!(Command::new("mount").args(["-t", "fusectl", "none"]).arg(dir.path()).status().unwrap().success());
        FuseCtl(dir)
    }
    fn ino(&self, conn: u64) -> Option<u64> {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(self.0.path().join(conn.to_string())).ok().map(|m| m.ino())
    }
}
impl Drop for FuseCtl {
    fn drop(&mut self) {
        let _ = Command::new("umount").arg(self.0.path()).status();
    }
}
struct Fixture { tmp: tempfile::TempDir, cas: Cas, tree: Tree }
fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir_all(tmp.path().join("src/pkg")).unwrap();
    fs::write(tmp.path().join("src/go.mod"), "module x\n").unwrap();
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas.import_dir(&tmp.path().join("src")).unwrap();
    Fixture { tmp, cas, tree }
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
    let f = fixture();
    let ctl = FuseCtl::mount();
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
