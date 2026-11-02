//! The lookup index: each manifest also stored as a tree of nodes, one per
//! directory its entries are in, content-addressed and shared between
//! manifests (`manifest_nodes`; `manifests.root` names a manifest's top).
//!
//! A node holds the entries whose parent is its directory and the ids of
//! its subdirectories' nodes, so its id covers every entry under it: two
//! manifests that read the same thing under a directory share that node.
//! A lookup checks candidates newest first and remembers each node's
//! verdict against the tree (`Eval`): a node is checked once whatever the
//! number of candidates holding it, and a candidate that differs from a
//! newer one only under the directories one commit touched costs only those
//! nodes. That makes a lookup proportional to what differs between the
//! manifests of a key, not to their number times their size (it decoded
//! every candidate in full before: ~30 ms and ~5 MB each on cli/cli).
//!
//! A node matches a tree exactly when every entry under it agrees with the
//! tree (`quod_manifest::agrees`, the rule `matches` applies entry by
//! entry), so a manifest's root matches exactly when `matches` says so.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use quod_cas::{Entry, Hash, Tree};
use quod_manifest::{Manifest, agrees};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::LedgerError;

/// One directory's part of a manifest.
#[derive(Debug, Serialize, Deserialize)]
struct Node {
    /// v1: entries and children, both sorted.
    v: u32,
    /// The manifest's entries whose parent is this directory, in manifest
    /// order (`.` is the top's).
    e: Vec<Entry>,
    /// Subdirectory name and its node's id.
    c: Vec<(String, Hash)>,
}

/// The directory an entry's node is: `""` for the top (and `.` itself).
fn dir_of(p: &str) -> &str {
    match p.rfind('/') {
        Some(i) => &p[..i],
        None => "",
    }
}

/// Stores `m`'s nodes (those not stored yet) and returns its root's id.
pub(crate) fn put_nodes(db: &Connection, m: &Manifest) -> Result<Hash, LedgerError> {
    // Every directory holding an entry, and every directory above one.
    let mut dirs: BTreeMap<&str, (Vec<&Entry>, BTreeSet<&str>)> = BTreeMap::new();
    dirs.entry("").or_default();
    for e in &m.entries {
        let d = if e.p == "." { "" } else { dir_of(&e.p) };
        dirs.entry(d).or_default().0.push(e);
        let mut child = d;
        // Up to the first directory already linked to its parent (every
        // one above it is too).
        while !child.is_empty() {
            let parent = dir_of(child);
            if !dirs.entry(parent).or_default().1.insert(child) {
                break;
            }
            child = parent;
        }
    }
    // Deepest first, so a node's children have ids before it.
    let mut order: Vec<&str> = dirs.keys().copied().collect();
    order.sort_by_key(|d| {
        std::cmp::Reverse(if d.is_empty() {
            0
        } else {
            d.matches('/').count() + 1
        })
    });
    let mut ids: HashMap<&str, Hash> = HashMap::new();
    let mut st =
        db.prepare_cached("INSERT OR IGNORE INTO manifest_nodes (id, body) VALUES (?1, ?2)")?;
    for d in order {
        let (entries, kids) = &dirs[d];
        let mut c: Vec<(String, Hash)> = kids
            .iter()
            .map(|k| {
                let name = if d.is_empty() { *k } else { &k[d.len() + 1..] };
                (name.to_string(), ids[k])
            })
            .collect();
        c.sort();
        let node = Node {
            v: 1,
            e: entries.iter().map(|e| (*e).clone()).collect(),
            c,
        };
        let body = quod_cas::canonical_json(&node).map_err(quod_manifest::ManifestError::from)?;
        let id = Hash::of(body.as_bytes());
        st.execute(params![id.to_string(), body.into_bytes()])?;
        ids.insert(d, id);
    }
    Ok(ids[""])
}

/// Node verdicts against one tree, remembered for one lookup.
pub(crate) struct Eval<'a> {
    db: &'a Connection,
    tree: &'a Tree,
    memo: HashMap<Hash, bool>,
    /// Nodes read from the ledger (for the stats).
    pub(crate) loaded: usize,
}

impl<'a> Eval<'a> {
    pub(crate) fn new(db: &'a Connection, tree: &'a Tree) -> Eval<'a> {
        Eval {
            db,
            tree,
            memo: HashMap::new(),
            loaded: 0,
        }
    }

    /// Whether every entry under node `id` agrees with the tree. A node
    /// the ledger does not have never matches.
    pub(crate) fn matches(&mut self, id: &Hash) -> Result<bool, LedgerError> {
        if let Some(&v) = self.memo.get(id) {
            return Ok(v);
        }
        let body: Option<Vec<u8>> = self
            .db
            .prepare_cached("SELECT body FROM manifest_nodes WHERE id = ?1")?
            .query_row([id.to_string()], |r| r.get(0))
            .optional()?;
        self.loaded += 1;
        let ok = match body {
            None => false,
            Some(b) => {
                let node: Node = serde_json::from_slice(&b)?;
                let mut ok = node.v == 1 && node.e.iter().all(|e| agrees(e, self.tree.get(&e.p)));
                if ok {
                    for (_, c) in &node.c {
                        if !self.matches(c)? {
                            ok = false;
                            break;
                        }
                    }
                }
                ok
            }
        };
        self.memo.insert(*id, ok);
        Ok(ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quod_manifest::matches;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE manifest_nodes (id TEXT PRIMARY KEY, body BLOB NOT NULL);")
            .unwrap();
        db
    }

    fn manifest(entries: Vec<Entry>) -> Manifest {
        let mut entries = entries;
        entries.sort_by(|a, b| a.p.as_bytes().cmp(b.p.as_bytes()));
        Manifest {
            v: quod_manifest::VERSION,
            step_key: Hash::of(b"k"),
            roots: vec!["/workspace".into()],
            entries,
            volatile: vec![],
        }
    }

    fn f(p: &str, c: &str) -> Entry {
        Entry::file(p, Hash::of(c.as_bytes()), 0o644)
    }

    #[test]
    fn a_root_matches_exactly_when_the_manifest_does() {
        let db = db();
        let tree = Tree::from_leaves([
            f("go.mod", "m"),
            f("a/b/c.go", "c"),
            f("a/d.go", "d"),
            f("x.go", "x"),
        ]);
        let cases = vec![
            manifest(vec![f("go.mod", "m"), f("a/b/c.go", "c")]),
            manifest(vec![f("go.mod", "m"), f("a/b/c.go", "C")]),
            manifest(vec![f("a/d.go", "d"), Entry::absent("a/b/e.go")]),
            manifest(vec![f("a/d.go", "d"), Entry::absent("a/b/c.go")]),
            manifest(vec![
                tree.get(".").unwrap().clone(),
                tree.get("a").unwrap().clone(),
            ]),
            manifest(vec![tree.get("a").unwrap().clone(), f("a/b/new.go", "n")]),
            manifest(vec![]),
        ];
        for m in &cases {
            let root = put_nodes(&db, m).unwrap();
            let mut ev = Eval::new(&db, &tree);
            assert_eq!(
                ev.matches(&root).unwrap(),
                matches(m, &tree).is_ok(),
                "{m:?}"
            );
        }
    }

    #[test]
    fn manifests_share_nodes_and_a_lookup_reads_each_once() {
        let db = db();
        let mut leaves: Vec<Entry> = (0..50)
            .flat_map(|d| (0..20).map(move |i| f(&format!("pkg{d}/f{i}.go"), "v1")))
            .collect();
        let tree = Tree::from_leaves(leaves.clone());
        let old = put_nodes(&db, &manifest(leaves.clone())).unwrap();
        leaves[7] = f("pkg0/f7.go", "v2");
        let new = put_nodes(&db, &manifest(leaves)).unwrap();
        let nodes: i64 = db
            .query_row("SELECT count(*) FROM manifest_nodes", [], |r| r.get(0))
            .unwrap();
        // 50 package nodes and a top for the first; one package and a top more.
        assert_eq!(nodes, 51 + 2);
        let mut ev = Eval::new(&db, &tree);
        // The new manifest fails at its first package: its top and pkg0.
        assert!(!ev.matches(&new).unwrap());
        assert_eq!(ev.loaded, 2);
        // The old one: its top, its own pkg0, and the 49 packages it shares
        // with the new one, which that check never reached.
        assert!(ev.matches(&old).unwrap());
        assert_eq!(ev.loaded, 2 + 51);
        // Asked again: remembered, nothing read.
        assert!(ev.matches(&old).unwrap() && !ev.matches(&new).unwrap());
        assert_eq!(ev.loaded, 53);
    }
}
