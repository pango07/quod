use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::Once;

use anyhow::Context;

/// Where Quod keeps its files.
pub struct State {
    pub dir: PathBuf,
}

impl State {
    pub fn resolve(flag: Option<&Path>) -> anyhow::Result<State> {
        let dir = match flag {
            Some(d) => d.to_path_buf(),
            None => match std::env::var_os("QUOD_HOME") {
                Some(d) => d.into(),
                None => {
                    let home = std::env::var_os("HOME")
                        .context("neither --state, QUOD_HOME nor HOME is set")?;
                    Path::new(&home).join(".cache/quod")
                }
            },
        };
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("create state dir {}", dir.display()))?;
        let state = State { dir };
        state.init_tracing()?;
        Ok(state)
    }

    pub fn cas_dir(&self) -> PathBuf {
        self.dir.join("cas")
    }

    pub fn ledger_path(&self) -> PathBuf {
        self.dir.join("ledger.sqlite")
    }

    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.dir.join("runs").join(run_id)
    }

    /// Claims `run_dir(run_id)` for this process: an exclusive lock on
    /// `runs/<id>.lock` held until the returned `RunDir` drops. Removes
    /// stale run directories first (`sweep_runs`). `keep`: the caller keeps
    /// the directory (`--keep`), which then outlives the claim on purpose.
    pub fn claim_run_dir(&self, run_id: &str, keep: bool) -> anyhow::Result<RunDir> {
        let runs = self.dir.join("runs");
        std::fs::create_dir_all(&runs).with_context(|| format!("create {}", runs.display()))?;
        sweep_runs(&runs);
        let lock_path = runs.join(format!("{run_id}.lock"));
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("open {}", lock_path.display()))?;
        lock.lock()
            .with_context(|| format!("lock {}", lock_path.display()))?;
        Ok(RunDir {
            path: self.run_dir(run_id),
            lock_path,
            _lock: lock,
            keep,
        })
    }

    pub fn events_path(&self) -> PathBuf {
        self.dir.join("events.jsonl")
    }

    /// The warm cache for steps pinned to `image` (the toolchain), named by
    /// the profile's cache variable (`GOCACHE` -> `gocache`): a persistent
    /// host directory the sandbox binds over that variable's directory.
    pub fn warm_cache_dir(&self, image: &quod_cas::Hash, cache_env: &str) -> PathBuf {
        self.dir
            .join("warm")
            .join(image.hex())
            .join(cache_env.to_ascii_lowercase())
    }

    /// A toolchain's download cache for setup steps (npm's, pip's, uv's):
    /// bound into --allow-net runs only, never part of a tree.
    pub fn setup_cache_dir(&self, toolchain: quod_key::Toolchain) -> PathBuf {
        self.dir.join("setup-cache").join(toolchain.name())
    }

    /// Structured events go to events.jsonl (the bench harness reads them),
    /// never to the step's stdout or stderr.
    fn init_tracing(&self) -> anyhow::Result<()> {
        static ONCE: Once = Once::new();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.events_path())
            .with_context(|| format!("open {}", self.events_path().display()))?;
        ONCE.call_once(|| {
            tracing_subscriber::fmt()
                .json()
                .flatten_event(true)
                .with_current_span(false)
                .with_writer(std::sync::Mutex::new(file))
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_env("QUOD_LOG")
                        .unwrap_or_else(|_| "quod=info,warn".into()),
                )
                .init();
        });
        Ok(())
    }
}

/// A run directory this process owns (`State::claim_run_dir`).
pub struct RunDir {
    pub path: PathBuf,
    lock_path: PathBuf,
    _lock: std::fs::File,
    keep: bool,
}

/// Marks a run directory kept on purpose (`--keep`): never swept.
const KEPT: &str = ".quod-kept";

impl Drop for RunDir {
    fn drop(&mut self) {
        if !self.path.exists() {
            let _ = std::fs::remove_file(&self.lock_path);
        } else if self.keep {
            let _ = std::fs::write(self.path.join(KEPT), b"");
        }
        // Else a run that failed before removing its directory: unlocked
        // once this drops, it is swept by the next run.
    }
}

/// Removes run directories no live process owns: one whose lock file can
/// be locked (its run ended without removing it, or died), that is not
/// marked kept. What such a run left mounted under it (a run killed with
/// SIGKILL, or by a machine that went down mid-run: its observer's FUSE
/// mount, the overlay over it, a rootfs overlay) is detached first, deepest
/// first; the directory goes only once nothing is mounted under it any
/// more. A live run holds its lock for as long as it has anything mounted,
/// so no live run's mount is ever touched. A run that crashed while
/// moving outputs into the store can leave a second name of a blob in its
/// upper dir (`Cas::take_files` links, then unlinks); nothing may ever
/// write through it, so it goes. Then the private fusectl mounts
/// (`quod_fs`'s ring teardown) of processes that are gone. Never fails a
/// run: problems are warnings. Without a readable mountinfo nothing is
/// swept: what is mounted under a run directory cannot be known, and
/// removing one through a mount would delete what the mount shows.
fn sweep_runs(runs: &Path) {
    sweep_runs_from(runs, mount_points, detach);
}

/// `sweep_runs` with mountinfo read by `mountinfo` and mounts detached by
/// `unmount`.
fn sweep_runs_from(
    runs: &Path,
    mountinfo: impl Fn() -> Option<Vec<PathBuf>>,
    unmount: impl Fn(&Path) -> std::io::Result<()>,
) {
    let Some(mounts) = mountinfo() else {
        tracing::warn!(target: "quod", "/proc/self/mountinfo unreadable; stale run directories left");
        return;
    };
    sweep_runs_with(runs, &mounts, &mountinfo, &unmount);
    if let Some(mounts) = mountinfo() {
        sweep_fusectl(&mounts, &std::env::temp_dir(), &unmount);
    }
}

/// Lazily unmounts `p` (umount2 with MNT_DETACH).
fn detach(p: &Path) -> std::io::Result<()> {
    let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes())?;
    // SAFETY: a valid NUL-terminated path that outlives the call.
    #[allow(unsafe_code)] // umount2(2) has no safe std wrapper
    let rc = unsafe { libc::umount2(c.as_ptr(), libc::MNT_DETACH) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Detaches the private fusectl mounts (`<tmp>/quod-fusectl-<pid>-<n>`)
/// whose process no longer exists, and removes their directories. A pid
/// that exists (ours, a live run's, or a reused one) is left alone.
fn sweep_fusectl(mounts: &[PathBuf], tmp: &Path, unmount: impl Fn(&Path) -> std::io::Result<()>) {
    let Ok(tmp) = tmp.canonicalize() else {
        return;
    };
    for m in mounts {
        let Some(name) = m
            .strip_prefix(&tmp)
            .ok()
            .filter(|r| r.components().count() == 1)
            .and_then(|r| r.to_str())
        else {
            continue;
        };
        let Some(pid) = name
            .strip_prefix("quod-fusectl-")
            .and_then(|r| r.split('-').next())
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        if Path::new(&format!("/proc/{pid}")).exists() {
            continue;
        }
        match unmount(m) {
            Ok(()) => {
                let _ = std::fs::remove_dir(m);
                tracing::info!(target: "quod", mount = %m.display(), "stale fusectl mount detached");
            }
            Err(e) => {
                tracing::warn!(target: "quod", mount = %m.display(), error = %e, "stale fusectl mount not detached");
            }
        }
    }
}

/// `sweep_runs` against a given list of mount points (as /proc lists them:
/// absolute, symlinks resolved), re-read with `remount` after detaching
/// with `unmount` (None: unreadable, so the directory stays). `runs` is
/// resolved the same way first, so a state dir given relative or through a
/// symlink still sees what is mounted under its run directories.
fn sweep_runs_with(
    runs: &Path,
    mounts: &[PathBuf],
    remount: impl Fn() -> Option<Vec<PathBuf>>,
    unmount: impl Fn(&Path) -> std::io::Result<()>,
) {
    let Ok(runs) = runs.canonicalize() else {
        return;
    };
    let runs = runs.as_path();
    let Ok(entries) = std::fs::read_dir(runs) else {
        return;
    };
    for e in entries.flatten() {
        let dir = e.path();
        let Some(id) = dir.file_name().and_then(|n| n.to_str()).map(String::from) else {
            continue;
        };
        if !dir.is_dir() || dir.join(KEPT).exists() {
            continue;
        }
        // A directory without a lock file predates claims: left alone.
        let lock_path = runs.join(format!("{id}.lock"));
        let Ok(lock) = OpenOptions::new().write(true).open(&lock_path) else {
            continue;
        };
        if lock.try_lock().is_err() {
            continue; // a live run
        }
        // Ours now (the lock is held until `lock` drops): what its dead
        // run left mounted under it, deepest first; a path with mounts
        // stacked on it is listed once per mount.
        let mut under: Vec<&PathBuf> = mounts.iter().filter(|m| m.starts_with(&dir)).collect();
        if !under.is_empty() {
            under.sort_by_key(|m| std::cmp::Reverse(m.components().count()));
            for m in under {
                if let Err(e) = unmount(m) {
                    tracing::warn!(target: "quod", mount = %m.display(), error = %e, "stale mount not detached");
                }
            }
            let left = remount().map_or(usize::MAX, |now| {
                now.into_iter().filter(|m| m.starts_with(&dir)).count()
            });
            if left > 0 {
                tracing::warn!(target: "quod", dir = %dir.display(), left, "stale run directory still has mounts; left");
                continue;
            }
            tracing::info!(target: "quod", dir = %dir.display(), "stale run directory's mounts detached");
        }
        match quod_cas::remove_any(&dir) {
            Ok(()) => {
                let _ = std::fs::remove_file(&lock_path);
                tracing::info!(target: "quod", dir = %dir.display(), "stale run directory removed");
            }
            Err(e) => {
                tracing::warn!(target: "quod", dir = %dir.display(), error = %e, "stale run directory not removed");
            }
        }
    }
}

/// Mount points from /proc/self/mountinfo, octal escapes decoded; None
/// when it cannot be read (or is empty: there is always a root mount).
fn mount_points() -> Option<Vec<PathBuf>> {
    let text = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    let points: Vec<PathBuf> = text
        .lines()
        .filter_map(|l| l.split(' ').nth(4))
        .map(|m| {
            let b = m.as_bytes();
            let mut out = Vec::with_capacity(b.len());
            let mut i = 0;
            while i < b.len() {
                let esc = (b[i] == b'\\' && i + 3 < b.len())
                    .then(|| std::str::from_utf8(&b[i + 1..i + 4]).ok())
                    .flatten()
                    .and_then(|o| u8::from_str_radix(o, 8).ok());
                match esc {
                    Some(c) => {
                        out.push(c);
                        i += 4;
                    }
                    None => {
                        out.push(b[i]);
                        i += 1;
                    }
                }
            }
            PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(out))
        })
        .collect();
    (!points.is_empty()).then_some(points)
}

#[cfg(test)]
mod run_dir_tests {
    use super::*;

    #[test]
    fn stale_run_dirs_go_and_live_or_kept_ones_stay() {
        let tmp = tempfile::tempdir().unwrap();
        let state = State {
            dir: tmp.path().to_path_buf(),
        };
        let live = state.claim_run_dir("live", false).unwrap();
        std::fs::create_dir_all(live.path.join("upper")).unwrap();
        // A run that died: its lock file is there, nobody holds it, and
        // its upper still has a blob's second name.
        let dead = state.claim_run_dir("dead", false).unwrap();
        std::fs::create_dir_all(dead.path.join("upper")).unwrap();
        std::fs::write(dead.path.join("upper/out"), b"x").unwrap();
        std::mem::forget(dead); // no Drop: as if killed
        std::fs::remove_file(tmp.path().join("runs/dead.lock")).ok();
        std::fs::write(tmp.path().join("runs/dead.lock"), b"").unwrap();
        // Kept on purpose.
        let kept = state.claim_run_dir("kept", true).unwrap();
        std::fs::create_dir_all(kept.path.join("upper")).unwrap();
        drop(kept);
        // Predates claims (no lock file).
        std::fs::create_dir_all(tmp.path().join("runs/old")).unwrap();

        let next = state.claim_run_dir("next", false).unwrap();
        let runs = tmp.path().join("runs");
        assert!(
            !runs.join("dead").exists(),
            "the dead run's directory is swept"
        );
        assert!(!runs.join("dead.lock").exists());
        assert!(
            runs.join("live/upper").exists(),
            "a live run's directory stays"
        );
        assert!(runs.join("kept/upper").exists(), "a kept one stays");
        assert!(runs.join("old").exists(), "one without a claim stays");
        drop(live);
        drop(next);
        assert!(
            runs.join("live.lock").exists(),
            "its directory still exists"
        );
        assert!(
            !runs.join("next.lock").exists(),
            "no directory, no lock file"
        );
    }

    /// A stale run directory with something mounted under it is left while
    /// the mount stays (its detach failed), also when the state dir is
    /// reached through a symlink or a path with `..` in it (mount points
    /// are absolute and resolved).
    #[test]
    fn a_mount_under_a_stale_run_dir_is_seen_through_a_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let state = State { dir: link.clone() };
        let dead = state.claim_run_dir("dead", false).unwrap();
        std::fs::create_dir_all(dead.path.join("merged")).unwrap();
        std::mem::forget(dead); // as if killed
        let lock = real.join("runs/dead.lock");
        std::fs::remove_file(&lock).unwrap();
        std::fs::write(&lock, b"").unwrap();
        // What /proc/self/mountinfo would list: the resolved path.
        let mounted = real.canonicalize().unwrap().join("runs/dead/merged");
        let stuck = || Some(vec![mounted.clone()]);
        let refuse = |_: &Path| Err(std::io::Error::from_raw_os_error(libc::EBUSY));
        for runs in [
            link.join("runs"),
            link.join("runs/../runs"),
            real.join("runs"),
        ] {
            sweep_runs_with(&runs, std::slice::from_ref(&mounted), stuck, refuse);
            assert!(
                real.join("runs/dead/merged").exists(),
                "swept a dir with a mount under it ({})",
                runs.display()
            );
        }
        // Unmounted, it goes.
        sweep_runs_with(&link.join("runs"), &[], || Some(Vec::new()), refuse);
        assert!(!real.join("runs/dead").exists());
    }

    /// What a killed run left mounted is detached, deepest first, and its
    /// directory goes; a live run's mounts are never touched.
    #[test]
    fn a_dead_runs_mounts_are_detached_and_a_live_runs_are_not() {
        let tmp = tempfile::tempdir().unwrap();
        let state = State {
            dir: tmp.path().to_path_buf(),
        };
        let live = state.claim_run_dir("live", false).unwrap();
        std::fs::create_dir_all(live.path.join("merged")).unwrap();
        let dead = state.claim_run_dir("dead", false).unwrap();
        std::fs::create_dir_all(dead.path.join("lower")).unwrap();
        std::fs::create_dir_all(dead.path.join("merged")).unwrap();
        std::mem::forget(dead); // as if killed with SIGKILL
        let runs = tmp.path().join("runs").canonicalize().unwrap();
        let lock = runs.join("dead.lock");
        std::fs::remove_file(&lock).unwrap();
        std::fs::write(&lock, b"").unwrap();
        // The overlay over the observer's mount, which is stacked twice.
        let mounts = vec![
            runs.join("dead/lower"),
            runs.join("dead/merged"),
            runs.join("dead/lower"),
            runs.join("live/merged"),
        ];
        let detached = std::cell::RefCell::new(Vec::new());
        sweep_runs_with(
            &runs,
            &mounts,
            || Some(vec![runs.join("live/merged")]),
            |p: &Path| {
                detached.borrow_mut().push(p.to_path_buf());
                Ok(())
            },
        );
        let d = detached.into_inner();
        assert_eq!(d.len(), 3, "{d:?}");
        assert!(d.iter().all(|p| p.starts_with(runs.join("dead"))), "{d:?}");
        assert!(!runs.join("dead").exists() && !lock.exists());
        assert!(runs.join("live/merged").exists());
        drop(live);
    }

    /// When mountinfo cannot be read, what is mounted under a stale run
    /// directory is unknown: nothing is swept (a remove would go through
    /// the mounts), neither before detaching nor after.
    #[test]
    fn an_unreadable_mountinfo_sweeps_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let state = State {
            dir: tmp.path().to_path_buf(),
        };
        let dead = state.claim_run_dir("dead", false).unwrap();
        std::fs::create_dir_all(dead.path.join("merged/data")).unwrap();
        std::mem::forget(dead); // as if killed
        let runs = tmp.path().join("runs").canonicalize().unwrap();
        let lock = runs.join("dead.lock");
        std::fs::remove_file(&lock).unwrap();
        std::fs::write(&lock, b"").unwrap();
        // Unreadable from the start.
        sweep_runs_from(&runs, || None, |_: &Path| Ok(()));
        assert!(runs.join("dead/merged/data").exists(), "swept blind");
        // Readable, a mount detached, then unreadable.
        let m = runs.join("dead/merged");
        let first = std::cell::Cell::new(true);
        sweep_runs_from(
            &runs,
            || first.replace(false).then(|| vec![m.clone()]),
            |_: &Path| Ok(()),
        );
        assert!(
            runs.join("dead/merged/data").exists(),
            "swept without seeing its mounts gone"
        );
        // Readable and nothing mounted: it goes.
        sweep_runs_from(&runs, || Some(Vec::new()), |_: &Path| Ok(()));
        assert!(!runs.join("dead").exists());
    }

    #[test]
    fn fusectl_mounts_of_gone_processes_are_detached() {
        let tmp = tempfile::tempdir().unwrap();
        let t = tmp.path().canonicalize().unwrap();
        let mine = t.join(format!("quod-fusectl-{}-0", std::process::id()));
        // pid_max is at most 2^22: this pid does not exist.
        let gone = t.join("quod-fusectl-4194999-3");
        let other = t.join("something-else");
        for d in [&mine, &gone, &other] {
            std::fs::create_dir(d).unwrap();
        }
        let detached = std::cell::RefCell::new(Vec::new());
        sweep_fusectl(
            &[mine.clone(), gone.clone(), other.clone()],
            &t,
            |p: &Path| {
                detached.borrow_mut().push(p.to_path_buf());
                Ok(())
            },
        );
        assert_eq!(detached.into_inner(), std::slice::from_ref(&gone));
        assert!(!gone.exists() && mine.exists() && other.exists());
    }
}
