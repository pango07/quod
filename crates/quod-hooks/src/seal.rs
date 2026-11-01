//! The sealed module cache: a subtree of the workspace served from an
//! immutable snapshot outside the observer and recorded whole. Shared by the
//! CLI's `quod run` and the hooks (see `snapshot` for the mechanism and
//! DECISIONS.md, exp/overhead, for why).

use std::path::Path;
use std::time::Instant;

use quod_cas::{Cas, Kind, Tree};
use quod_manifest::Access;

use crate::snapshot;

/// A subtree served from a snapshot outside the observer and recorded whole.
pub struct Sealed {
    /// Workspace-relative path of the subtree.
    pub prefix: String,
    /// A directory under it that stays observed (the step writes there).
    pub carve_out: String,
    /// The snapshot, shared-locked while held; the subtree is at
    /// `snapshot.dir/prefix`.
    pub snapshot: snapshot::Snapshot,
}

/// Seals the module cache at `modcache` (a GOMODCACHE value) when it lies
/// under `workspace` and the tree has it with its `cache/` directory (so go
/// has somewhere to write): the step reads an immutable snapshot of exactly
/// the imported content, built once per distinct subtree under `snapshots`
/// and reused across runs and commits. `None`, with a warning, when no
/// snapshot could be made: the module cache is then observed like
/// everything else. `enabled` false turns sealing off (the CLI's
/// QUOD_SEAL_MODCACHE=0, the bench's attribution series).
pub fn seal_modcache(
    modcache: &str,
    workspace: &str,
    tree: &Tree,
    cas: &Cas,
    snapshots: &Path,
    enabled: bool,
) -> Option<Sealed> {
    if !enabled {
        return None;
    }
    let prefix = modcache
        .strip_prefix(workspace)?
        .strip_prefix('/')?
        .trim_end_matches('/')
        .to_string();
    let carve_out = format!("{prefix}/cache");
    let has_dir = |p: &str| tree.get(p).is_some_and(|e| e.k == Kind::Dir);
    if !has_dir(&prefix) || !has_dir(&carve_out) {
        return None;
    }
    // The carve-out directory itself is in the snapshot (empty) as the
    // mount point; its contents are not part of the key.
    let (under, carve) = (format!("{prefix}/"), format!("{carve_out}/"));
    let entries: Vec<&quod_cas::Entry> = tree
        .iter()
        .filter(|e| (e.p == prefix || e.p.starts_with(&under)) && !e.p.starts_with(&carve))
        .collect();
    let t = Instant::now();
    match snapshot::ensure(cas, &entries, snapshots) {
        Ok(s) => {
            tracing::info!(target: "quod", key = %s.key, reused = s.reused, entries = entries.len(), ms = t.elapsed().as_millis() as u64, "snapshot");
            Some(Sealed {
                prefix,
                carve_out,
                snapshot: s,
            })
        }
        Err(e) => {
            tracing::warn!(error = %e, "module cache not sealed; observing it instead");
            None
        }
    }
}

/// Every tree entry at or under `prefix`, except those under `carve_out`,
/// as accesses: what a sealed subtree records. Over-records by construction:
/// any change under the prefix alters a recorded file, symlink or listing.
pub fn sealed_accesses(tree: &Tree, prefix: &str, carve_out: &str) -> Vec<Access> {
    let (under, carve) = (format!("{prefix}/"), format!("{carve_out}/"));
    tree.iter()
        .filter(|e| e.p == prefix || e.p.starts_with(&under))
        .filter(|e| e.p != carve_out && !e.p.starts_with(&carve))
        .map(|e| Access {
            path: e.p.clone(),
            kind: e.k,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use quod_cas::{Entry, Hash};

    #[test]
    fn sealed_accesses_cover_the_prefix_minus_the_carve_out() {
        let tree = Tree::from_leaves([
            Entry::file("go.mod", Hash::of(b"m"), 0o644),
            Entry::file(".quod/gomod/a.b/c@v1/x.go", Hash::of(b"x"), 0o444),
            Entry::symlink(".quod/gomod/a.b/c@v1/l", "x.go"),
            Entry::file(
                ".quod/gomod/cache/download/a.b/c/@v/v1.mod",
                Hash::of(b"d"),
                0o444,
            ),
            Entry::file(".quod/gomodx/y", Hash::of(b"y"), 0o644),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        for b in [b"m".as_slice(), b"x", b"d", b"y"] {
            cas.put(b).unwrap();
        }
        let root = tmp.path().join("snapshots");
        let s = seal_modcache(
            "/workspace/.quod/gomod",
            "/workspace",
            &tree,
            &cas,
            &root,
            true,
        )
        .expect("GOMODCACHE is under the workspace");
        assert_eq!(
            (s.prefix.as_str(), s.carve_out.as_str()),
            (".quod/gomod", ".quod/gomod/cache")
        );
        // The snapshot holds the module dirs and an empty carve-out to mount over.
        let snap = &s.snapshot.dir;
        assert_eq!(snap.parent(), Some(root.as_path()));
        assert_eq!(
            std::fs::read(snap.join(".quod/gomod/a.b/c@v1/x.go")).unwrap(),
            b"x"
        );
        assert_eq!(
            std::fs::read_dir(snap.join(".quod/gomod/cache"))
                .unwrap()
                .count(),
            0
        );
        assert!(!snap.join("go.mod").exists() && !snap.join(".quod/gomodx").exists());
        // A second tree with the same module cache reuses it.
        let s2 = seal_modcache(
            "/workspace/.quod/gomod",
            "/workspace",
            &tree,
            &cas,
            &root,
            true,
        )
        .unwrap();
        assert_eq!(&s2.snapshot.dir, snap);
        // Outside the workspace, or without the cache dir: not sealed.
        assert!(
            seal_modcache("/elsewhere/gomod", "/workspace", &tree, &cas, &root, true).is_none()
        );
        assert!(
            seal_modcache(
                "/workspace/.quod/gomodx",
                "/workspace",
                &tree,
                &cas,
                &root,
                true
            )
            .is_none()
        );
        let got: Vec<(String, Kind)> = sealed_accesses(&tree, &s.prefix, &s.carve_out)
            .into_iter()
            .map(|a| (a.path, a.kind))
            .collect();
        let want: Vec<(String, Kind)> = [
            (".quod/gomod", Kind::Dir),
            (".quod/gomod/a.b", Kind::Dir),
            (".quod/gomod/a.b/c@v1", Kind::Dir),
            (".quod/gomod/a.b/c@v1/l", Kind::Symlink),
            (".quod/gomod/a.b/c@v1/x.go", Kind::File),
        ]
        .iter()
        .map(|(p, k)| (p.to_string(), *k))
        .collect();
        assert_eq!(got, want);
    }
}
