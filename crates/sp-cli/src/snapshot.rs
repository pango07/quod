//! Immutable snapshots of a sealed subtree, built once per distinct subtree
//! and reused across runs and commits, by any process sharing the store.
//!
//! A snapshot is keyed by a hash over its entries (path, kind, content hash,
//! mode, target; each field length-prefixed, the format versioned), so two
//! trees whose sealed subtrees agree share one, and any difference, down to
//! a mode bit, gets another. It is built under a temporary name, made
//! durable with `syncfs`, marked complete, and only then renamed into place:
//! a key directory without the marker is never used (a crash left it torn
//! or half deleted) and is replaced. Its directories are read-only and its
//! files hard links to CAS blobs (or copies where the mode differs; see
//! `Cas::link_entries`), and the store never writes into an existing blob,
//! so once built it cannot change. The step gets it as a read-only bind.
//!
//! Every user holds a shared lock on `<root>/<key>.lock` from `ensure` until
//! it drops the `Snapshot`; gc takes the exclusive lock without waiting and
//! leaves busy snapshots alone, and removes a snapshot by renaming it out of
//! the way first, so a crash mid-removal cannot leave a half-deleted
//! snapshot under its key.

use std::fs::{self, File, FileTimes};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use sp_cas::{Cas, Entry, Hash, Kind};

/// Snapshots kept per store beyond the one in use.
pub const KEEP: usize = 4;

/// Partial builds older than this are leftovers of a crashed run.
const STALE_BUILD: Duration = Duration::from_secs(3600);

/// Written last into a finished build; its absence means "not a snapshot".
const MARKER: &str = ".complete";

/// Changes to what the key covers, how it is spelled, or what a snapshot
/// built under it looks like bump this. v2: file and directory times are
/// content-derived and fixed (`set_times`), not the host's.
const KEY_FORMAT: &[u8] = b"stillpass snapshot key v2\0";

pub struct Snapshot {
    /// The snapshot's directory; the subtree lies under it at its own paths.
    pub dir: PathBuf,
    pub key: Hash,
    /// Found already built by an earlier run.
    pub reused: bool,
    /// The shared lock that keeps gc away while this snapshot is in use.
    _lock: File,
}

/// BLAKE3 over the format version and every entry's path, kind, hash, mode
/// and target, each variable field length-prefixed so no spelling of one
/// entry can collide with another.
pub fn key<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Hash {
    let mut buf = Vec::with_capacity(1 << 16);
    buf.extend_from_slice(KEY_FORMAT);
    let field = |buf: &mut Vec<u8>, b: &[u8]| {
        buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
        buf.extend_from_slice(b);
    };
    for e in entries {
        field(&mut buf, e.p.as_bytes());
        buf.push(e.k.letter() as u8);
        field(
            &mut buf,
            e.h.map(|h| h.hex()).unwrap_or_default().as_bytes(),
        );
        buf.push(e.m.is_some() as u8);
        buf.extend_from_slice(&e.m.unwrap_or(0).to_le_bytes());
        buf.push(e.t.is_some() as u8);
        field(&mut buf, e.t.as_deref().unwrap_or("").as_bytes());
    }
    Hash::of(&buf)
}

/// The snapshot of `entries` under `root`, built if no run built it before.
/// The returned `Snapshot` holds it shared-locked until dropped.
pub fn ensure(cas: &Cas, entries: &[&Entry], root: &Path) -> anyhow::Result<Snapshot> {
    fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
    let key = key(entries.iter().copied());
    let dir = root.join(key.hex());
    let lock = lock_shared(&lock_path(root, &key))?;
    if complete(&dir) {
        touch(&dir);
        return Ok(Snapshot {
            dir,
            key,
            reused: true,
            _lock: lock,
        });
    }
    if dir.exists() {
        // A key directory without its marker: torn by a crash. Out of the
        // way first, so nothing can mistake it for a snapshot meanwhile.
        let gone = root.join(format!("tmp.gc.{}.{}", key.hex(), random()));
        if fs::rename(&dir, &gone).is_ok() {
            let _ = sp_cas::remove_any(&gone);
        }
    }
    let tmp = root.join(format!("tmp.{}.{}", key.hex(), random()));
    if let Err(e) = build(cas, entries, &tmp, root) {
        let _ = sp_cas::remove_any(&tmp);
        return Err(e);
    }
    match fs::rename(&tmp, &dir) {
        Ok(()) => Ok(Snapshot {
            dir,
            key,
            reused: false,
            _lock: lock,
        }),
        // Another process finished the same snapshot first; its copy is
        // as good as ours.
        Err(_) if complete(&dir) => {
            let _ = sp_cas::remove_any(&tmp);
            Ok(Snapshot {
                dir,
                key,
                reused: true,
                _lock: lock,
            })
        }
        Err(e) => {
            let _ = sp_cas::remove_any(&tmp);
            Err(e).with_context(|| format!("move snapshot into {}", dir.display()))
        }
    }
}

/// Links the entries into `tmp`, makes them durable, marks the build
/// complete (durably), then makes the directories read-only.
fn build(cas: &Cas, entries: &[&Entry], tmp: &Path, root: &Path) -> anyhow::Result<()> {
    sp_cas::remove_any(tmp)?;
    cas.link_entries(entries.iter().copied(), tmp)?;
    set_times(entries, tmp).with_context(|| format!("set times in {}", tmp.display()))?;
    // Everything the marker vouches for is on disk before the marker is.
    sp_fs::syncfs(root).with_context(|| format!("syncfs {}", root.display()))?;
    let marker = tmp.join(MARKER);
    let mut m = File::create(&marker).with_context(|| format!("create {}", marker.display()))?;
    m.write_all(b"complete\n")?;
    m.sync_all()?;
    seal_dirs(tmp)?;
    File::open(tmp)?.sync_all()?;
    Ok(())
}

/// Every file gets the time the observer serves for its content
/// (`sp_fs::content_mtime`) and every directory a fixed one, so a sealed
/// tree shows no host-specific mtimes: what a step sees under a sealed
/// prefix is then a function of the snapshot key. A blob's inode is shared
/// by every snapshot that links it, and its content-derived time is the same
/// for all of them. Symlinks keep theirs (std has no lutimes; nothing seen
/// in the corpus reads a symlink's own mtime). Directory order is still the
/// host filesystem's (see DECISIONS).
fn set_times(entries: &[&Entry], tmp: &Path) -> io::Result<()> {
    for e in entries {
        let t = match (e.k, e.h) {
            (Kind::File, Some(h)) => sp_fs::content_mtime(&h),
            (Kind::Dir, _) => UNIX_EPOCH,
            _ => continue,
        };
        File::open(tmp.join(&e.p))?.set_times(FileTimes::new().set_accessed(t).set_modified(t))?;
    }
    Ok(())
}

fn complete(dir: &Path) -> bool {
    dir.join(MARKER).is_file()
}

fn lock_path(root: &Path, key: &Hash) -> PathBuf {
    root.join(format!("{}.lock", key.hex()))
}

/// Opens (creating) the lock file and takes a shared lock on it, retrying
/// if a gc unlinked the file between the open and the lock: a lock on an
/// unlinked file guards nothing.
fn lock_shared(path: &Path) -> anyhow::Result<File> {
    loop {
        let f = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .with_context(|| format!("open {}", path.display()))?;
        f.lock_shared()
            .with_context(|| format!("lock {}", path.display()))?;
        if same_inode(&f, path) {
            return Ok(f);
        }
    }
}

fn same_inode(f: &File, path: &Path) -> bool {
    match (f.metadata(), fs::metadata(path)) {
        (Ok(a), Ok(b)) => a.ino() == b.ino() && a.dev() == b.dev(),
        _ => false,
    }
}

/// Removes leftovers and all but the `keep` most recently used snapshots
/// (`current` and any snapshot another run holds locked always stay).
/// Never fails: a completed run is not undone by housekeeping, so every
/// problem is a warning. Returns how many snapshots went.
pub fn gc(root: &Path, keep: usize, current: &Path) -> usize {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    let now = SystemTime::now();
    let mut removed = 0;
    let mut snapshots: Vec<(SystemTime, PathBuf)> = Vec::new();
    for item in entries.flatten() {
        let path = item.path();
        let name = item.file_name().to_string_lossy().into_owned();
        // Another process may rename or remove things while we look.
        let Ok(mtime) = item.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if name.starts_with("tmp.gc.") {
            // Being removed, by us earlier or by a run that died doing it.
            let _ = sp_cas::remove_any(&path);
        } else if name.starts_with("tmp.") {
            if now.duration_since(mtime).unwrap_or_default() > STALE_BUILD {
                let _ = sp_cas::remove_any(&path);
            }
        } else if name.ends_with(".lock") {
            // Removed with its snapshot.
        } else if path != current && item.file_type().is_ok_and(|t| t.is_dir()) {
            snapshots.push((mtime, path));
        }
    }
    snapshots.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, dir) in snapshots.into_iter().skip(keep.saturating_sub(1)) {
        match remove_snapshot(root, &dir) {
            Ok(true) => removed += 1,
            Ok(false) => {}
            Err(e) => tracing::warn!(snapshot = %dir.display(), error = %e, "snapshot gc"),
        }
    }
    removed
}

/// Removes one snapshot unless a run holds it: exclusive lock without
/// waiting, rename out of the way, then delete. `Ok(false)` when busy.
fn remove_snapshot(root: &Path, dir: &Path) -> io::Result<bool> {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let lock_path = root.join(format!("{name}.lock"));
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(false),
        Err(std::fs::TryLockError::Error(e)) => return Err(e),
    }
    if !same_inode(&lock, &lock_path) {
        // Someone else removed and recreated it under us; leave it alone.
        return Ok(false);
    }
    let gone = root.join(format!("tmp.gc.{name}.{}", random()));
    match fs::rename(dir, &gone) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    }
    let _ = fs::remove_file(&lock_path);
    sp_cas::remove_any(&gone).map_err(io::Error::other)?;
    Ok(true)
}

/// Marks a snapshot as used now (gc keeps the most recently used).
fn touch(dir: &Path) {
    if let Ok(f) = File::open(dir) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// A suffix no other process will pick: not the pid, which pid namespaces
/// sharing a volume reuse.
pub(crate) fn random() -> String {
    let mut b = [0u8; 8];
    if File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_err()
    {
        let t = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        b = (t ^ (std::process::id() as u64).rotate_left(32)).to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Every directory read-only: the snapshot is complete and must not change.
fn seal_dirs(dir: &Path) -> io::Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        if item.file_type()?.is_dir() {
            seal_dirs(&item.path())?;
        }
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(0o555))
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use sp_cas::{Kind, Tree};

    use super::*;

    const PREFIX: &str = ".stillpass/gomod";

    /// A tree with a module cache of two files and a symlink; `mode` is the
    /// mode of `a.go`, `content` its content.
    fn tree(mode: u32, content: &[u8]) -> Tree {
        Tree::from_leaves([
            Entry::file("go.mod", Hash::of(b"module x"), 0o644),
            Entry::file(".stillpass/gomod/a.b/c@v1/a.go", Hash::of(content), mode),
            Entry::file(".stillpass/gomod/a.b/c@v1/b.go", Hash::of(b"b"), 0o444),
            Entry::symlink(".stillpass/gomod/a.b/c@v1/l", "a.go"),
            Entry::file(
                ".stillpass/gomod/cache/download/a.b/c/@v/v1.mod",
                Hash::of(b"d"),
                0o444,
            ),
        ])
    }

    fn sealed(t: &Tree) -> Vec<&Entry> {
        t.iter()
            .filter(|e| e.p == PREFIX || e.p.starts_with(".stillpass/gomod/"))
            .filter(|e| !e.p.starts_with(".stillpass/gomod/cache/"))
            .collect()
    }

    fn cas(tmp: &Path) -> Cas {
        let cas = Cas::open(tmp.join("cas")).unwrap();
        for b in [b"module x".as_slice(), b"a", b"a2", b"b", b"d"] {
            cas.put(b).unwrap();
        }
        cas
    }

    fn snapshot_dirs(root: &Path) -> usize {
        fs::read_dir(root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_type().unwrap().is_dir())
            .count()
    }

    #[test]
    fn same_subtree_reuses_the_snapshot_and_never_rebuilds() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = cas(tmp.path());
        let root = tmp.path().join("snapshots");
        let t = tree(0o444, b"a");
        let first = ensure(&cas, &sealed(&t), &root).unwrap();
        assert!(!first.reused);
        let a = first.dir.join(PREFIX).join("a.b/c@v1/a.go");
        let blob = cas.blob_path(&Hash::of(b"a"));
        assert_eq!(
            fs::metadata(&a).unwrap().ino(),
            fs::metadata(&blob).unwrap().ino(),
            "a read-only file is a hard link to its blob"
        );
        assert_eq!(
            fs::metadata(first.dir.join(PREFIX)).unwrap().mode() & 0o777,
            0o555
        );
        assert!(first.dir.join(PREFIX).join("cache").is_dir(), "mount point");
        assert!(complete(&first.dir) && !first.dir.join("go.mod").exists());

        // A different tree with the same sealed subtree finds it.
        let other = tree(0o444, b"a").with_leaf_changed(Entry::file(
            "go.mod",
            Hash::of(b"module y"),
            0o644,
        ));
        let again = ensure(&cas, &sealed(&other), &root).unwrap();
        assert!(again.reused);
        assert_eq!(again.dir, first.dir);
        assert_eq!(again.key, first.key);
        assert_eq!(snapshot_dirs(&root), 1);
    }

    #[test]
    fn any_difference_in_the_subtree_is_another_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = cas(tmp.path());
        let root = tmp.path().join("snapshots");
        let base = tree(0o444, b"a");
        let k0 = ensure(&cas, &sealed(&base), &root).unwrap();
        let variants = [
            tree(0o444, b"a2"), // content
            tree(0o644, b"a"),  // mode
            base.with_outputs(&[Entry::file(
                ".stillpass/gomod/a.b/c@v1/new.go",
                Hash::of(b"b"),
                0o444,
            )]), // an added file
            base.with_leaf_changed(Entry::symlink(".stillpass/gomod/a.b/c@v1/l", "b.go")), // target
        ];
        let mut keys = vec![k0.key];
        for v in &variants {
            let s = ensure(&cas, &sealed(v), &root).unwrap();
            assert!(!s.reused, "{:?}", s.key);
            assert!(!keys.contains(&s.key));
            keys.push(s.key);
        }
        // The carve-out's contents are not part of the key.
        let carve = base.with_leaf_changed(Entry::file(
            ".stillpass/gomod/cache/download/a.b/c/@v/v1.mod",
            Hash::of(b"a2"),
            0o444,
        ));
        assert!(ensure(&cas, &sealed(&carve), &root).unwrap().reused);
        // The first snapshot is untouched by all of that.
        let a = k0.dir.join(PREFIX).join("a.b/c@v1/a.go");
        assert_eq!(fs::read(&a).unwrap(), b"a");
        assert_eq!(fs::metadata(&a).unwrap().mode() & 0o777, 0o444);
    }

    #[test]
    fn sealed_files_show_content_times_not_the_hosts() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = cas(tmp.path());
        let root = tmp.path().join("snapshots");
        // a.go is copied (0644 against a 0444 blob), b.go hard-linked.
        let s = ensure(&cas, &sealed(&tree(0o644, b"a")), &root).unwrap();
        let mtime = |p: PathBuf| fs::metadata(p).unwrap().modified().unwrap();
        let dir = s.dir.join(PREFIX).join("a.b/c@v1");
        assert_eq!(
            mtime(dir.join("a.go")),
            sp_fs::content_mtime(&Hash::of(b"a"))
        );
        assert_eq!(
            mtime(dir.join("b.go")),
            sp_fs::content_mtime(&Hash::of(b"b"))
        );
        assert_eq!(mtime(dir.clone()), UNIX_EPOCH);
        assert_eq!(mtime(s.dir.join(PREFIX).join("cache")), UNIX_EPOCH);
    }

    #[test]
    fn key_fields_cannot_run_into_each_other() {
        // Moving bytes between a path and a symlink target, or between two
        // entries, changes the key.
        let a = [
            Entry::symlink("x/ab", "c"),
            Entry::file("y", Hash::of(b"q"), 0o644),
        ];
        let b = [
            Entry::symlink("x/a", "bc"),
            Entry::file("y", Hash::of(b"q"), 0o644),
        ];
        assert_ne!(key(a.iter()), key(b.iter()));
        let c = [Entry::symlink("x/ab", "c\0y")];
        let d = [Entry::symlink("x/ab", "c"), Entry::absent("y")];
        assert_ne!(key(c.iter()), key(d.iter()));
    }

    #[test]
    fn modes_never_leak_between_snapshots_sharing_a_blob() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = cas(tmp.path());
        let root = tmp.path().join("snapshots");
        let ro = ensure(&cas, &sealed(&tree(0o444, b"a")), &root).unwrap();
        let rw = ensure(&cas, &sealed(&tree(0o644, b"a")), &root).unwrap();
        let (a_ro, a_rw) = (
            ro.dir.join(PREFIX).join("a.b/c@v1/a.go"),
            rw.dir.join(PREFIX).join("a.b/c@v1/a.go"),
        );
        let blob = fs::metadata(cas.blob_path(&Hash::of(b"a"))).unwrap();
        assert_eq!(fs::metadata(&a_ro).unwrap().mode() & 0o777, 0o444);
        assert_eq!(fs::metadata(&a_rw).unwrap().mode() & 0o777, 0o644);
        assert_eq!(fs::metadata(&a_ro).unwrap().ino(), blob.ino(), "linked");
        assert_ne!(fs::metadata(&a_rw).unwrap().ino(), blob.ino(), "copied");
        assert_eq!(blob.mode() & 0o777, 0o444, "the blob's mode is untouched");
        assert_eq!(fs::read(&a_rw).unwrap(), b"a");
    }

    #[test]
    fn torn_and_partial_snapshots_are_never_used() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = cas(tmp.path());
        let root = tmp.path().join("snapshots");
        let t = tree(0o444, b"a");
        let k = key(sealed(&t));
        fs::create_dir_all(&root).unwrap();
        // A crashed run left a partial build under the same key, and a
        // crashed gc left a key directory without its marker.
        let partial = root.join(format!("tmp.{}.deadbeef", k.hex()));
        fs::create_dir_all(partial.join(PREFIX)).unwrap();
        fs::write(partial.join(PREFIX).join("junk"), b"x").unwrap();
        let torn = root.join(k.hex());
        fs::create_dir_all(torn.join(PREFIX)).unwrap();
        fs::write(torn.join(PREFIX).join("junk"), b"x").unwrap();
        let s = ensure(&cas, &sealed(&t), &root).unwrap();
        assert!(!s.reused, "a key directory without its marker is rebuilt");
        assert!(complete(&s.dir));
        assert!(!s.dir.join(PREFIX).join("junk").exists());
        assert!(s.dir.join(PREFIX).join("a.b/c@v1/a.go").is_file());
        assert!(partial.exists(), "a fresh partial build is left alone");
        File::open(&partial)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(7200))
            .unwrap();
        gc(&root, KEEP, &s.dir);
        assert!(!partial.exists());
        assert_eq!(snapshot_dirs(&root), 1);
    }

    #[test]
    fn gc_keeps_the_most_recently_used_and_never_a_busy_one() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = cas(tmp.path());
        let root = tmp.path().join("snapshots");
        let mut dirs = Vec::new();
        let mut held = None;
        for (i, mode) in [0o444u32, 0o644, 0o600, 0o640, 0o660, 0o664]
            .iter()
            .enumerate()
        {
            let s = ensure(&cas, &sealed(&tree(*mode, b"a")), &root).unwrap();
            // Distinct, increasing mtimes.
            File::open(&s.dir)
                .unwrap()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1000 + i as u64))
                .unwrap();
            dirs.push(s.dir.clone());
            // The second oldest stays in use (another run would hold it).
            if i == 1 {
                held = Some(s);
            }
        }
        // Reuse the oldest: it becomes the most recently used.
        let s = ensure(&cas, &sealed(&tree(0o444, b"a")), &root).unwrap();
        assert!(s.reused && s.dir == dirs[0]);
        drop(s);
        let current = dirs[5].clone();
        // Besides current, the three most recently used stay: the just-used
        // oldest (0), 4 and 3. Of the two victims, 1 is busy and survives.
        assert_eq!(gc(&root, 4, &current), 1);
        let left: Vec<bool> = dirs.iter().map(|d| d.is_dir()).collect();
        assert_eq!(left, [true, true, false, true, true, true], "{left:?}");
        drop(held);
        assert_eq!(gc(&root, 4, &current), 1, "released, it can go");
        assert!(!dirs[1].is_dir());
        assert!(matches!(
            tree(0o444, b"a").get(PREFIX).map(|e| e.k),
            Some(Kind::Dir)
        ));
    }
}
