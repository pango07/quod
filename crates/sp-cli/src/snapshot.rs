//! Immutable snapshots of a sealed subtree, built once per distinct subtree
//! and reused across runs and commits.
//!
//! A snapshot is keyed by a hash over its entries (path, kind, content hash,
//! mode, target), so two trees whose sealed subtrees agree share one, and
//! any difference, down to a mode bit, gets another. It is built under a
//! temporary name and renamed into place, so a partial build is never
//! visible under a key; its directories are read-only and its files hard
//! links to CAS blobs (or copies where the mode differs; see
//! `Cas::link_entries`), and the store never writes into an existing blob,
//! so once built it cannot change. The step gets it as a read-only bind.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Context;
use sp_cas::{Cas, Entry, Hash};

/// Snapshots kept per state directory beyond the one in use.
pub const KEEP: usize = 4;

/// Partial builds older than this are leftovers of a crashed run.
const STALE_BUILD: Duration = Duration::from_secs(3600);

pub struct Snapshot {
    /// The snapshot's directory; the subtree lies under it at its own paths.
    pub dir: PathBuf,
    pub key: Hash,
    /// Found already built by an earlier run.
    pub reused: bool,
}

/// BLAKE3 over every entry's path, kind, hash, mode and target, in order.
pub fn key<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> Hash {
    let mut buf = String::new();
    for e in entries {
        buf.push_str(&e.p);
        buf.push('\t');
        buf.push(e.k.letter());
        buf.push('\t');
        if let Some(h) = &e.h {
            buf.push_str(&h.hex());
        }
        buf.push('\t');
        if let Some(m) = e.m {
            buf.push_str(&format!("{m:o}"));
        }
        buf.push('\t');
        if let Some(t) = &e.t {
            buf.push_str(t);
        }
        buf.push('\n');
    }
    Hash::of(buf.as_bytes())
}

/// The snapshot of `entries` under `root`, built if no run built it before.
pub fn ensure(cas: &Cas, entries: &[&Entry], root: &Path) -> anyhow::Result<Snapshot> {
    let key = key(entries.iter().copied());
    let dir = root.join(key.hex());
    if dir.is_dir() {
        touch(&dir);
        return Ok(Snapshot {
            dir,
            key,
            reused: true,
        });
    }
    fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
    let tmp = root.join(format!("tmp.{}.{}", key.hex(), std::process::id()));
    sp_cas::remove_any(&tmp)?;
    let built = cas
        .link_entries(entries.iter().copied(), &tmp)
        .and_then(|()| seal_dirs(&tmp));
    if let Err(e) = built {
        let _ = sp_cas::remove_any(&tmp);
        return Err(e.into());
    }
    match fs::rename(&tmp, &dir) {
        Ok(()) => Ok(Snapshot {
            dir,
            key,
            reused: false,
        }),
        // Another process finished the same snapshot first; its copy is
        // as good as ours.
        Err(_) if dir.is_dir() => {
            let _ = sp_cas::remove_any(&tmp);
            Ok(Snapshot {
                dir,
                key,
                reused: true,
            })
        }
        Err(e) => {
            let _ = sp_cas::remove_any(&tmp);
            Err(e).with_context(|| format!("move snapshot into {}", dir.display()))
        }
    }
}

/// Removes partial builds older than an hour and all but the `keep` most
/// recently used snapshots (`current` always stays). Returns how many went.
pub fn gc(root: &Path, keep: usize, current: &Path) -> anyhow::Result<usize> {
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(0);
    };
    let now = SystemTime::now();
    let mut removed = 0;
    let mut snapshots: Vec<(SystemTime, PathBuf)> = Vec::new();
    for item in entries {
        let item = item?;
        let path = item.path();
        let mtime = item.metadata()?.modified()?;
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with("tmp.") {
            if now.duration_since(mtime).unwrap_or_default() > STALE_BUILD {
                sp_cas::remove_any(&path)?;
                removed += 1;
            }
        } else if path != current {
            snapshots.push((mtime, path));
        }
    }
    snapshots.sort_by_key(|a| std::cmp::Reverse(a.0));
    for (_, path) in snapshots.into_iter().skip(keep.saturating_sub(1)) {
        sp_cas::remove_any(&path)?;
        removed += 1;
    }
    Ok(removed)
}

/// Marks a snapshot as used now (gc keeps the most recently used).
fn touch(dir: &Path) {
    if let Ok(f) = fs::File::open(dir) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// Every directory read-only: the snapshot is complete and must not change.
fn seal_dirs(dir: &Path) -> Result<(), sp_cas::CasError> {
    for item in fs::read_dir(dir).map_err(|e| io_at(dir, e))? {
        let item = item.map_err(|e| io_at(dir, e))?;
        if item.file_type().map_err(|e| io_at(dir, e))?.is_dir() {
            seal_dirs(&item.path())?;
        }
    }
    fs::set_permissions(dir, fs::Permissions::from_mode(0o555)).map_err(|e| io_at(dir, e))
}

fn io_at(path: &Path, source: std::io::Error) -> sp_cas::CasError {
    sp_cas::CasError::Io {
        path: path.to_path_buf(),
        source,
    }
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
        assert!(!first.dir.join("go.mod").exists());

        // A different tree with the same sealed subtree finds it.
        let mut other = tree(0o444, b"a");
        other = other.with_leaf_changed(Entry::file("go.mod", Hash::of(b"module y"), 0o644));
        let again = ensure(&cas, &sealed(&other), &root).unwrap();
        assert!(again.reused);
        assert_eq!(again.dir, first.dir);
        assert_eq!(again.key, first.key);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
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
    fn partial_builds_are_never_used_and_old_ones_are_collected() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = cas(tmp.path());
        let root = tmp.path().join("snapshots");
        let t = tree(0o444, b"a");
        let k = key(sealed(&t));
        // A crashed run left a partial build under the same key.
        let partial = root.join(format!("tmp.{}.999", k.hex()));
        fs::create_dir_all(partial.join(PREFIX)).unwrap();
        fs::write(partial.join(PREFIX).join("junk"), b"x").unwrap();
        let s = ensure(&cas, &sealed(&t), &root).unwrap();
        assert!(!s.reused);
        assert!(!s.dir.join(PREFIX).join("junk").exists());
        assert!(partial.exists(), "a fresh partial build is left alone");
        fs::File::open(&partial)
            .unwrap()
            .set_modified(SystemTime::now() - Duration::from_secs(7200))
            .unwrap();
        assert_eq!(gc(&root, KEEP, &s.dir).unwrap(), 1);
        assert!(!partial.exists());
    }

    #[test]
    fn gc_keeps_the_most_recently_used() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = cas(tmp.path());
        let root = tmp.path().join("snapshots");
        let mut dirs = Vec::new();
        for (i, mode) in [0o444u32, 0o644, 0o600, 0o640, 0o660, 0o664]
            .iter()
            .enumerate()
        {
            let s = ensure(&cas, &sealed(&tree(*mode, b"a")), &root).unwrap();
            // Distinct, increasing mtimes.
            fs::File::open(&s.dir)
                .unwrap()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1000 + i as u64))
                .unwrap();
            dirs.push(s.dir);
        }
        // Reuse the oldest: it becomes the most recently used.
        let s = ensure(&cas, &sealed(&tree(0o444, b"a")), &root).unwrap();
        assert!(s.reused && s.dir == dirs[0]);
        let current = dirs[5].clone();
        assert_eq!(gc(&root, 4, &current).unwrap(), 2);
        let left: Vec<bool> = dirs.iter().map(|d| d.is_dir()).collect();
        assert_eq!(left, [true, false, false, true, true, true]);
        assert!(matches!(
            tree(0o444, b"a").get(PREFIX).map(|e| e.k),
            Some(Kind::Dir)
        ));
    }
}
