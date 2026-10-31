use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Hash;

/// The path of the tree root.
pub const ROOT: &str = ".";

/// Entry kinds. `f`, `d` and `l` describe paths that exist; `a` records a
/// lookup that found nothing; `x` marks a path a step deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Kind {
    #[serde(rename = "f")]
    File,
    #[serde(rename = "d")]
    Dir,
    #[serde(rename = "a")]
    Absent,
    #[serde(rename = "l")]
    Symlink,
    #[serde(rename = "x")]
    Deleted,
}

impl Kind {
    pub fn letter(self) -> char {
        match self {
            Kind::File => 'f',
            Kind::Dir => 'd',
            Kind::Absent => 'a',
            Kind::Symlink => 'l',
            Kind::Deleted => 'x',
        }
    }
}

/// One path. The field names are the schema (`p`, `k`, `h`, `m`, `t`).
///
/// `f`: content hash and mode masked to 0o777. `d`: hash of the listing.
/// `l`: target string. `a` and `x`: path only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub p: String,
    pub k: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<Hash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub m: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<String>,
}

impl Entry {
    pub fn file(p: impl Into<String>, h: Hash, mode: u32) -> Entry {
        Entry {
            p: p.into(),
            k: Kind::File,
            h: Some(h),
            m: Some(mode & 0o777),
            t: None,
        }
    }

    pub fn dir(p: impl Into<String>, listing: Hash) -> Entry {
        Entry {
            p: p.into(),
            k: Kind::Dir,
            h: Some(listing),
            m: None,
            t: None,
        }
    }

    pub fn symlink(p: impl Into<String>, target: impl Into<String>) -> Entry {
        Entry {
            p: p.into(),
            k: Kind::Symlink,
            h: None,
            m: None,
            t: Some(target.into()),
        }
    }

    pub fn absent(p: impl Into<String>) -> Entry {
        Entry {
            p: p.into(),
            k: Kind::Absent,
            h: None,
            m: None,
            t: None,
        }
    }

    pub fn deleted(p: impl Into<String>) -> Entry {
        Entry {
            p: p.into(),
            k: Kind::Deleted,
            h: None,
            m: None,
            t: None,
        }
    }
}

/// Hash of a directory listing: one `name<TAB>kind<LF>` line per child,
/// sorted bytewise by name. Unambiguous only because names containing a tab
/// or newline are refused everywhere a tree is built (see `valid_name`).
pub fn listing_hash<'a>(children: impl IntoIterator<Item = (&'a str, Kind)>) -> Hash {
    let mut kids: Vec<(&str, Kind)> = children.into_iter().collect();
    kids.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut buf = String::new();
    for (name, kind) in kids {
        buf.push_str(name);
        buf.push('\t');
        buf.push(kind.letter());
        buf.push('\n');
    }
    Hash::of(buf.as_bytes())
}

/// Names a tree may contain: UTF-8 (by type) and free of the listing
/// separators, so two different listings never hash alike.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['\t', '\n', '/', '\0'])
}

/// `parent/name`, with the root spelled `.`.
pub fn child_path(parent: &str, name: &str) -> String {
    if parent == ROOT {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// Splits `a/b/c` into (`a/b`, `c`) and `c` into (`.`, `c`).
pub fn parent_and_name(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => (ROOT, path),
    }
}

/// A materialized workspace: every path, as `f`, `d` or `l` entries, keyed by
/// relative path (the root is `.`). Iteration is bytewise path order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    entries: BTreeMap<String, Entry>,
}

impl Tree {
    /// Builds a tree from file and symlink entries. Parent directories are
    /// created as needed and every directory's listing hash is computed.
    pub fn from_leaves(leaves: impl IntoIterator<Item = Entry>) -> Tree {
        let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
        let mut dirs: BTreeMap<String, Vec<(String, Kind)>> = BTreeMap::new();
        dirs.insert(ROOT.to_string(), Vec::new());
        let add_child = |dirs: &mut BTreeMap<String, Vec<(String, Kind)>>, path: &str, kind| {
            let mut path = path.to_string();
            let mut kind = kind;
            loop {
                let (parent, name) = parent_and_name(&path);
                let (parent, name) = (parent.to_string(), name.to_string());
                let fresh = !dirs.contains_key(&parent);
                let kids = dirs.entry(parent.clone()).or_default();
                if !kids.iter().any(|(n, _)| *n == name) {
                    kids.push((name, kind));
                }
                if !fresh || parent == ROOT {
                    break;
                }
                path = parent;
                kind = Kind::Dir;
            }
        };
        for e in leaves {
            match e.k {
                Kind::Dir => {
                    if e.p != ROOT {
                        add_child(&mut dirs, &e.p, Kind::Dir);
                    }
                    dirs.entry(e.p.clone()).or_default();
                }
                Kind::File | Kind::Symlink => {
                    add_child(&mut dirs, &e.p, e.k);
                    entries.insert(e.p.clone(), e);
                }
                Kind::Absent | Kind::Deleted => {}
            }
        }
        for (path, kids) in dirs {
            if let Some(leaf) = entries.get(&path) {
                panic!(
                    "tree invariant: {:?} is both a {:?} and a directory",
                    path, leaf.k
                );
            }
            let h = listing_hash(kids.iter().map(|(n, k)| (n.as_str(), *k)));
            entries.insert(path.clone(), Entry::dir(path, h));
        }
        Tree { entries }
    }

    pub fn get(&self, path: &str) -> Option<&Entry> {
        self.entries.get(path)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Direct children of `dir`, in bytewise order: (name, entry).
    pub fn children<'a>(&'a self, dir: &str) -> impl Iterator<Item = (&'a str, &'a Entry)> {
        let prefix = if dir == ROOT {
            String::new()
        } else {
            format!("{dir}/")
        };
        let start = prefix.clone();
        self.entries
            .range(start..)
            .take_while(move |(p, _)| p.starts_with(&prefix) || prefix.is_empty())
            .filter_map(move |(p, e)| {
                if p == ROOT {
                    return None;
                }
                let rest = &p[if dir == ROOT { 0 } else { dir.len() + 1 }..];
                (!rest.contains('/')).then_some((rest, e))
            })
    }

    /// A copy without the top-level `name` (and everything under it), with the
    /// root listing recomputed. Used to drop `.git` from the served tree.
    pub fn without_top_level(&self, name: &str) -> Tree {
        let prefix = format!("{name}/");
        Tree::from_leaves(
            self.entries
                .values()
                .filter(|e| e.p != name && !e.p.starts_with(&prefix))
                .cloned(),
        )
    }

    /// A copy with one existing file or symlink's content replaced. Listings
    /// are unaffected (same name, same kind), so nothing is recomputed.
    pub fn with_leaf_changed(&self, e: Entry) -> Tree {
        let old = self
            .entries
            .get(&e.p)
            .expect("with_leaf_changed on a missing path");
        assert!(
            old.k == e.k && matches!(e.k, Kind::File | Kind::Symlink),
            "with_leaf_changed must keep the kind"
        );
        let mut entries = self.entries.clone();
        entries.insert(e.p.clone(), e);
        Tree { entries }
    }

    /// The tree after applying `outputs` (f, l and x entries) on top of it.
    pub fn with_outputs<'a>(&self, outputs: impl IntoIterator<Item = &'a Entry>) -> Tree {
        let mut leaves: BTreeMap<String, Entry> = self.entries.clone();
        for o in outputs {
            match o.k {
                Kind::Deleted => {
                    let prefix = format!("{}/", o.p);
                    leaves.retain(|p, _| p != &o.p && !p.starts_with(&prefix));
                }
                Kind::File | Kind::Symlink => {
                    // Overlayfs writes no whiteout when a file replaces a
                    // directory, so the directory's contents go here.
                    let prefix = format!("{}/", o.p);
                    leaves.retain(|p, _| !p.starts_with(&prefix));
                    leaves.insert(o.p.clone(), o.clone());
                }
                Kind::Dir => {
                    leaves.insert(o.p.clone(), o.clone());
                }
                Kind::Absent => {}
            }
        }
        Tree::from_leaves(leaves.into_values())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Tree {
        Tree::from_leaves([
            Entry::file("a/b/c.txt", Hash::of(b"c"), 0o644),
            Entry::file("a/z", Hash::of(b"z"), 0o100755),
            Entry::symlink("ln", "a/z"),
            Entry::file("a.b", Hash::of(b"ab"), 0o600),
            Entry::dir("empty", Hash::of(b"")),
        ])
    }

    #[test]
    fn parents_and_listings() {
        let t = t();
        let paths: Vec<&str> = t.iter().map(|e| e.p.as_str()).collect();
        assert_eq!(
            paths,
            [".", "a", "a.b", "a/b", "a/b/c.txt", "a/z", "empty", "ln"]
        );
        assert_eq!(t.get("a/z").unwrap().m, Some(0o755));
        let root = listing_hash([
            ("a", Kind::Dir),
            ("a.b", Kind::File),
            ("empty", Kind::Dir),
            ("ln", Kind::Symlink),
        ]);
        assert_eq!(t.get(".").unwrap().h, Some(root));
        assert_eq!(t.get("empty").unwrap().h, Some(Hash::of(b"")));
        assert_eq!(t.get("a").unwrap().h, Some(Hash::of(b"b\td\nz\tf\n")));
    }

    #[test]
    fn children_are_direct_only() {
        let t = t();
        let root: Vec<&str> = t.children(ROOT).map(|(n, _)| n).collect();
        assert_eq!(root, ["a", "a.b", "empty", "ln"]);
        let a: Vec<&str> = t.children("a").map(|(n, _)| n).collect();
        assert_eq!(a, ["b", "z"]);
        assert_eq!(t.children("empty").count(), 0);
    }

    #[test]
    fn drop_top_level() {
        let t = t().without_top_level("a");
        assert!(t.get("a").is_none() && t.get("a/b/c.txt").is_none());
        assert!(t.get("a.b").is_some());
    }

    #[test]
    fn file_replacing_a_directory_drops_its_contents() {
        let t = t().with_outputs(&[Entry::file("a", Hash::of(b"x"), 0o644)]);
        assert_eq!(t.get("a").unwrap().k, Kind::File);
        assert!(t.get("a/b/c.txt").is_none() && t.get("a/b").is_none());
    }

    #[test]
    #[should_panic(expected = "both")]
    fn leaf_and_directory_at_one_path_is_refused() {
        Tree::from_leaves([
            Entry::file("a", Hash::of(b"x"), 0o644),
            Entry::file("a/b", Hash::of(b"y"), 0o644),
        ]);
    }

    #[test]
    fn apply_outputs() {
        let t = t().with_outputs(&[
            Entry::deleted("a/b"),
            Entry::file("new/x", Hash::of(b"x"), 0o644),
        ]);
        assert!(t.get("a/b").is_none() && t.get("a/b/c.txt").is_none());
        assert!(t.get("new").is_some_and(|e| e.k == Kind::Dir));
        assert_eq!(t.get("a").unwrap().h, Some(Hash::of(b"z\tf\n")));
    }
}
