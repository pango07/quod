//! Sealed prefixes: dependency trees the step reads from an immutable
//! snapshot outside the observer, recorded whole. Which prefixes, and
//! which directories under them the tool writes to (carve-outs that stay
//! on the observed overlay), is the toolchain profile's to say; this module
//! turns a profile's `Seal`s into snapshots, sandbox mounts and manifest
//! accesses the same way for every toolchain.
//!
//! Soundness, per sealed prefix: the snapshot is exactly the imported
//! content (hard links to blobs, see `snapshot`), bound read-only, so the
//! step can see nothing but what the tree says; and every tree entry under
//! the prefix except the carve-outs is recorded, so any content, mode,
//! target, addition or removal under it changes a recorded entry. Absent
//! lookups under the prefix need no separate record: the listing they
//! happened in is recorded. Carve-outs are observed like the rest of the
//! workspace; a carve-out the tree lacks is created empty, in the snapshot
//! (so there is a mount point) and in the merged dir (so there is a source).

use std::path::Path;
use std::time::Instant;

use quod_cas::{Cas, Entry, Hash, Tree};
use quod_key::{Profile, Seal};
use quod_manifest::Access;
use quod_sandbox::Mount;

use crate::snapshot;

/// A sealed prefix with its snapshot, shared-locked while held.
pub struct Sealed {
    pub seal: Seal,
    pub snapshot: snapshot::Snapshot,
}

/// Is sealing turned off? QUOD_SEAL=0 (or the older
/// QUOD_SEAL_MODCACHE=0) serves every prefix through the observer
/// instead, for the bench's attribution series.
pub fn disabled() -> bool {
    ["QUOD_SEAL", "QUOD_SEAL_MODCACHE"]
        .iter()
        .any(|v| std::env::var(v).is_ok_and(|x| x == "0"))
}

/// Seals every prefix the profile finds in the tree. A prefix whose
/// snapshot cannot be made is observed like everything else, with a
/// warning; nothing fails.
pub fn seal(profile: &Profile, tree: &Tree, cas: &Cas, snapshots: &Path) -> Vec<Sealed> {
    if disabled() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for seal in profile.seals(tree) {
        let entries = snapshot_entries(tree, &seal);
        let refs: Vec<&Entry> = entries.iter().collect();
        let t = Instant::now();
        match snapshot::ensure(cas, &refs, snapshots) {
            Ok(s) => {
                tracing::info!(target: "quod", prefix = %seal.prefix, key = %s.key, reused = s.reused, entries = refs.len(), ms = t.elapsed().as_millis() as u64, "snapshot");
                out.push(Sealed { seal, snapshot: s });
            }
            Err(e) => {
                tracing::warn!(prefix = %seal.prefix, error = %e, "prefix not sealed; observing it instead");
            }
        }
    }
    out
}

/// What goes into the snapshot: every tree entry at or under the prefix
/// except what is under a carve-out, plus each carve-out as an empty
/// directory (the mount point; its contents are outside the key and the
/// snapshot, whether or not the tree has it).
fn snapshot_entries(tree: &Tree, seal: &Seal) -> Vec<Entry> {
    let mut entries: Vec<Entry> = tree
        .iter()
        .filter(|e| in_prefix(&e.p, &seal.prefix))
        .filter(|e| {
            !seal
                .carve_outs
                .iter()
                .any(|c| e.p == *c || in_prefix(&e.p, c))
        })
        .cloned()
        .collect();
    for c in &seal.carve_outs {
        entries.push(Entry::dir(c.clone(), Hash::of(b"")));
    }
    entries.sort_by(|a, b| a.p.as_bytes().cmp(b.p.as_bytes()));
    entries
}

fn in_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || (path.len() > prefix.len()
            && path.starts_with(prefix)
            && path.as_bytes()[prefix.len()] == b'/')
}

/// The sandbox mounts for one sealed prefix: the snapshot read-only over
/// the prefix, then each carve-out bound back from the merged dir (observed
/// and writable). Creates a carve-out the merged dir lacks.
pub fn mounts(sealed: &Sealed, merged: &Path) -> std::io::Result<Vec<Mount>> {
    let ws = Path::new(quod_key::WORKSPACE);
    let mut out = vec![Mount {
        src: sealed.snapshot.dir.join(&sealed.seal.prefix),
        dst: ws.join(&sealed.seal.prefix),
        writable: false,
    }];
    for c in &sealed.seal.carve_outs {
        let src = merged.join(c);
        std::fs::create_dir_all(&src)?;
        out.push(Mount {
            src,
            dst: ws.join(c),
            writable: true,
        });
    }
    Ok(out)
}

/// Every tree entry at or under the prefix, except the carve-outs and what
/// is under them, as accesses: what a sealed prefix records. Over-records
/// by construction: any change under the prefix alters a recorded file,
/// symlink or listing.
pub fn accesses(tree: &Tree, seal: &Seal) -> Vec<Access> {
    tree.iter()
        .filter(|e| in_prefix(&e.p, &seal.prefix))
        .filter(|e| {
            !seal
                .carve_outs
                .iter()
                .any(|c| e.p == *c || in_prefix(&e.p, c))
        })
        .map(|e| Access {
            path: e.p.clone(),
            kind: e.k,
        })
        .collect()
}

/// `fuse+sealed` style suffix for the run event.
pub fn label(sealed: &[Sealed]) -> &'static str {
    if sealed.is_empty() { "" } else { "+sealed" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quod_cas::Kind;
    use quod_key::Toolchain;

    fn go_tree() -> Tree {
        Tree::from_leaves([
            Entry::file("go.mod", Hash::of(b"m"), 0o644),
            Entry::file(".quod/gomod/a.b/c@v1/x.go", Hash::of(b"x"), 0o444),
            Entry::symlink(".quod/gomod/a.b/c@v1/l", "x.go"),
            Entry::file(
                ".quod/gomod/cache/download/a.b/c/@v/v1.mod",
                Hash::of(b"d"),
                0o444,
            ),
            Entry::file(".quod/gomodx/y", Hash::of(b"y"), 0o644),
        ])
    }

    fn cas(tmp: &Path) -> Cas {
        let cas = Cas::open(tmp.join("cas")).unwrap();
        for b in [b"m".as_slice(), b"x", b"d", b"y", b"a", b"v"] {
            cas.put(b).unwrap();
        }
        cas
    }

    #[test]
    fn go_module_cache_is_sealed_minus_its_carve_out() {
        let tree = go_tree();
        let tmp = tempfile::tempdir().unwrap();
        let cas = cas(tmp.path());
        let root = tmp.path().join("snapshots");
        let sealed = seal(Toolchain::Go.profile(), &tree, &cas, &root);
        assert_eq!(sealed.len(), 1);
        let s = &sealed[0];
        assert_eq!(s.seal.prefix, ".quod/gomod");
        assert_eq!(s.seal.carve_outs, [".quod/gomod/cache"]);
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
        // A second tree with the same module cache reuses it, whatever is
        // under the carve-out.
        let other = tree.with_outputs(&[Entry::file(
            ".quod/gomod/cache/download/new",
            Hash::of(b"v"),
            0o444,
        )]);
        let s2 = seal(Toolchain::Go.profile(), &other, &cas, &root);
        assert_eq!(s2[0].snapshot.dir, *snap);
        assert!(s2[0].snapshot.reused);
        let got: Vec<(String, Kind)> = accesses(&tree, &s.seal)
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
        // Mounts: the snapshot read-only, the carve-out writable from merged.
        let merged = tmp.path().join("merged");
        std::fs::create_dir_all(&merged).unwrap();
        let m = mounts(s, &merged).unwrap();
        assert_eq!(m.len(), 2);
        assert!(!m[0].writable && m[0].dst == Path::new("/workspace/.quod/gomod"));
        assert!(m[1].writable && m[1].src == merged.join(".quod/gomod/cache"));
        assert!(
            merged.join(".quod/gomod/cache").is_dir(),
            "created for the bind"
        );
    }

    #[test]
    fn node_modules_carve_outs_exist_in_the_snapshot_without_being_in_the_tree() {
        let tree = Tree::from_leaves([
            Entry::file("package.json", Hash::of(b"m"), 0o644),
            Entry::file("node_modules/a/index.js", Hash::of(b"a"), 0o644),
            Entry::file("node_modules/.vite/results.json", Hash::of(b"v"), 0o644),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let cas = cas(tmp.path());
        let root = tmp.path().join("snapshots");
        let sealed = seal(Toolchain::Node.profile(), &tree, &cas, &root);
        assert_eq!(sealed.len(), 1);
        let s = &sealed[0];
        let snap = &s.snapshot.dir;
        for c in [".cache", ".vite", ".vite-temp", ".vitest"] {
            let d = snap.join("node_modules").join(c);
            assert!(d.is_dir(), "{c}");
            assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0, "{c} is empty");
        }
        assert_eq!(
            std::fs::read(snap.join("node_modules/a/index.js")).unwrap(),
            b"a"
        );
        // The carve-out's contents are not recorded by the seal (they are
        // observed), and the carve-out itself is not either.
        let paths: Vec<String> = accesses(&tree, &s.seal)
            .into_iter()
            .map(|a| a.path)
            .collect();
        assert_eq!(
            paths,
            ["node_modules", "node_modules/a", "node_modules/a/index.js"]
        );
        // A tree without the carve-out gets the same empty mount points. Its
        // key differs only because the prefix's own listing (a recorded
        // entry) names `.vite` in one tree and not the other.
        let bare = Tree::from_leaves([
            Entry::file("package.json", Hash::of(b"m"), 0o644),
            Entry::file("node_modules/a/index.js", Hash::of(b"a"), 0o644),
        ]);
        let s2 = seal(Toolchain::Node.profile(), &bare, &cas, &root);
        assert!(s2[0].snapshot.dir.join("node_modules/.vite").is_dir());
        assert!(!s2[0].snapshot.reused);
        let again = seal(Toolchain::Node.profile(), &bare, &cas, &root);
        assert!(again[0].snapshot.reused && again[0].snapshot.key == s2[0].snapshot.key);
    }

    #[test]
    fn prefix_matching_is_by_path_component() {
        assert!(in_prefix("a/b", "a"));
        assert!(in_prefix("a", "a"));
        assert!(!in_prefix("ab", "a"));
        assert!(!in_prefix("a.b/c", "a"));
    }

    #[test]
    fn nothing_to_seal_without_the_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let cas = cas(tmp.path());
        let tree = Tree::from_leaves([Entry::file("go.mod", Hash::of(b"m"), 0o644)]);
        assert!(seal(Toolchain::Go.profile(), &tree, &cas, &tmp.path().join("s")).is_empty());
    }
}
