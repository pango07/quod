//! Manifest and step result types, serialization, and `matches(manifest,
//! tree)` with a structured mismatch reason. Pure functions; no storage, no
//! execution.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};
use sp_cas::{CanonError, canonical_json};
pub use sp_cas::{Entry, Hash, Kind, Tree};

/// Schema version of manifests and results. v2 (2026-10-31): results gain
/// `root_outputs`, what a step wrote outside the workspace when it ran in a
/// pinned rootfs (the whole root is a per-step overlay there).
pub const VERSION: u32 = 2;

/// One observed access, as the observer logs it. `kind` is what the access
/// proved about the path: `f`, `d` or `l` for a path that exists, `a` for a
/// lookup that found nothing.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Access {
    pub path: String,
    pub kind: Kind,
}

/// What one execution of a step actually read. Entries are sorted bytewise by
/// path (then kind) and unique.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub v: u32,
    pub step_key: Hash,
    pub roots: Vec<String>,
    pub entries: Vec<Entry>,
}

/// What a step produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepResult {
    pub v: u32,
    pub step_key: Hash,
    pub manifest_id: Hash,
    pub run_id: String,
    pub recorded_at: String,
    pub exit_code: i32,
    pub stdout: Hash,
    pub stderr: Hash,
    /// Writes under the workspace root, relative to it.
    pub outputs: Vec<Entry>,
    /// Writes anywhere else, relative to `/`: in a pinned rootfs the whole
    /// root is an overlay and its upper dir is diffed like the workspace's
    /// (a tool cache under /opt, a config under /home). Empty on the host
    /// root, where writes outside the workspace fail. Left out of the JSON
    /// when empty, so results without them read as before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub root_outputs: Vec<Entry>,
    pub duration_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("access log names {0:?}, which is not in the served tree")]
    UnknownPath(String),
    #[error("access log records {kind:?} for {path:?}, which the tree has as {actual:?}")]
    KindConflict {
        path: String,
        kind: Kind,
        actual: Kind,
    },
    #[error(transparent)]
    Canon(#[from] CanonError),
}

/// The first manifest entry the tree disagrees with, and how many disagree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub path: String,
    pub expected: Entry,
    /// What the tree has at that path, if anything.
    pub actual: Option<Entry>,
    pub total: usize,
}

impl Manifest {
    /// Builds a manifest from the observer's access log. Each access is
    /// resolved against the tree that was served; duplicates collapse.
    ///
    /// Conservative on conflict: an access whose kind disagrees with the tree
    /// is an error, never silently reclassified, and an `a` for a path the
    /// tree has is kept alongside the tree's entry (it can never match, so
    /// the step can never replay on a wrong manifest).
    pub fn from_log(
        step_key: Hash,
        roots: Vec<String>,
        tree: &Tree,
        log: impl IntoIterator<Item = Access>,
    ) -> Result<Manifest, ManifestError> {
        let mut seen: BTreeSet<(String, Kind)> = BTreeSet::new();
        let mut entries = Vec::new();
        for a in log {
            if !seen.insert((a.path.clone(), a.kind)) {
                continue;
            }
            match (a.kind, tree.get(&a.path)) {
                (Kind::Absent, None) => entries.push(Entry::absent(a.path)),
                (Kind::Absent, Some(e)) => {
                    entries.push(Entry::absent(a.path.clone()));
                    if seen.insert((a.path, e.k)) {
                        entries.push(e.clone());
                    }
                }
                (_, None) => return Err(ManifestError::UnknownPath(a.path)),
                (k, Some(e)) if k == e.k => entries.push(e.clone()),
                (k, Some(e)) => {
                    return Err(ManifestError::KindConflict {
                        path: a.path,
                        kind: k,
                        actual: e.k,
                    });
                }
            }
        }
        entries.sort_by(|a, b| a.p.as_bytes().cmp(b.p.as_bytes()).then(a.k.cmp(&b.k)));
        Ok(Manifest {
            v: VERSION,
            step_key,
            roots,
            entries,
        })
    }

    pub fn canonical_json(&self) -> Result<String, ManifestError> {
        Ok(canonical_json(self)?)
    }
}

/// Over-records: adds a read of every regular file whose parent directory the
/// log already names (as `d`) and whose name satisfies `pick`, so a tool that
/// might consult such files without the observer seeing it (a cached verdict
/// standing in for a run that read them) still has them in the manifest.
/// Paths the tree does not have as files are left alone. Idempotent.
pub fn record_children(tree: &Tree, log: &mut Vec<Access>, pick: impl Fn(&str) -> bool) {
    let dirs: BTreeSet<&str> = log
        .iter()
        .filter(|a| a.kind == Kind::Dir)
        .map(|a| a.path.as_str())
        .collect();
    let mut extra = Vec::new();
    for dir in dirs {
        for (name, e) in tree.children(dir) {
            if e.k == Kind::File && pick(name) {
                extra.push(Access {
                    path: e.p.clone(),
                    kind: Kind::File,
                });
            }
        }
    }
    log.extend(extra);
}

pub fn manifest_id(m: &Manifest) -> Result<Hash, ManifestError> {
    Ok(Hash::of(m.canonical_json()?.as_bytes()))
}

/// Does `tree` agree with every entry of `manifest`?
///
/// `f`: same hash and mode. `d`: same listing hash. `a`: no entry. `l`: same
/// target. Paths the manifest doesn't mention are irrelevant.
pub fn matches(manifest: &Manifest, tree: &Tree) -> Result<(), Box<Mismatch>> {
    let mut first: Option<(String, Entry, Option<Entry>)> = None;
    let mut total = 0;
    for want in &manifest.entries {
        let have = tree.get(&want.p);
        if !entry_agrees(want, have) {
            total += 1;
            if first.is_none() {
                first = Some((want.p.clone(), want.clone(), have.cloned()));
            }
        }
    }
    match first {
        None => Ok(()),
        Some((path, expected, actual)) => Err(Box::new(Mismatch {
            path,
            expected,
            actual,
            total,
        })),
    }
}

fn entry_agrees(want: &Entry, have: Option<&Entry>) -> bool {
    match (want.k, have) {
        (Kind::Absent, None) => true,
        (Kind::Absent, Some(_)) | (_, None) => false,
        (Kind::File, Some(h)) => h.k == Kind::File && h.h == want.h && h.m == want.m,
        (Kind::Dir, Some(h)) => h.k == Kind::Dir && h.h == want.h,
        (Kind::Symlink, Some(h)) => h.k == Kind::Symlink && h.t == want.t,
        (Kind::Deleted, Some(_)) => false,
    }
}

fn describe(e: &Entry) -> String {
    let short = |h: &Option<Hash>| h.map(|h| format!("b3:{}", h.short())).unwrap_or_default();
    match e.k {
        Kind::File => format!("file {} mode {:o}", short(&e.h), e.m.unwrap_or(0)),
        Kind::Dir => format!("directory listing {}", short(&e.h)),
        Kind::Symlink => format!("symlink -> {}", e.t.as_deref().unwrap_or("")),
        Kind::Absent => "absent".to_string(),
        Kind::Deleted => "deleted".to_string(),
    }
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match (self.expected.k, &self.actual) {
            (Kind::Dir, Some(a)) if a.k == Kind::Dir => "directory listing changed".to_string(),
            (Kind::File, Some(a)) if a.k == Kind::File && a.h == self.expected.h => {
                "file mode changed".to_string()
            }
            (Kind::File, Some(a)) if a.k == Kind::File => "file content changed".to_string(),
            (Kind::Absent, Some(_)) => "was absent, now present".to_string(),
            (_, None) => "was present, now absent".to_string(),
            _ => "kind changed".to_string(),
        };
        let actual = self.actual.as_ref().map_or("absent".to_string(), describe);
        write!(
            f,
            "{}: {what} ({} -> {actual})",
            self.path,
            describe(&self.expected)
        )?;
        if self.total > 1 {
            write!(f, ", {} more", self.total - 1)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Tree {
        Tree::from_leaves([
            Entry::file("go.mod", Hash::of(b"module x"), 0o644),
            Entry::file("pkg/cmd/root.go", Hash::of(b"package cmd"), 0o644),
            Entry::symlink("bin/gh", "../build/gh"),
        ])
    }

    fn log(items: &[(&str, Kind)]) -> Vec<Access> {
        items
            .iter()
            .map(|(p, k)| Access {
                path: p.to_string(),
                kind: *k,
            })
            .collect()
    }

    #[test]
    fn from_log_dedups_sorts_and_resolves() {
        let t = tree();
        let m = Manifest::from_log(
            Hash::of(b"k"),
            vec!["/workspace".into()],
            &t,
            log(&[
                ("pkg/cmd/root.go", Kind::File),
                ("go.mod", Kind::File),
                ("pkg/cmd", Kind::Dir),
                ("go.mod", Kind::File),
                ("internal/x/testdata/missing.json", Kind::Absent),
                ("bin/gh", Kind::Symlink),
            ]),
        )
        .unwrap();
        let paths: Vec<&str> = m.entries.iter().map(|e| e.p.as_str()).collect();
        assert_eq!(
            paths,
            [
                "bin/gh",
                "go.mod",
                "internal/x/testdata/missing.json",
                "pkg/cmd",
                "pkg/cmd/root.go"
            ]
        );
        assert_eq!(m.entries[1], *t.get("go.mod").unwrap());
        assert!(matches(&m, &t).is_ok());
    }

    #[test]
    fn from_log_rejects_unknown_and_conflicting() {
        let t = tree();
        let k = Hash::of(b"k");
        assert!(matches!(
            Manifest::from_log(k, vec![], &t, log(&[("nope", Kind::File)])),
            Err(ManifestError::UnknownPath(_))
        ));
        assert!(matches!(
            Manifest::from_log(k, vec![], &t, log(&[("go.mod", Kind::Dir)])),
            Err(ManifestError::KindConflict { .. })
        ));
        let m = Manifest::from_log(k, vec![], &t, log(&[("go.mod", Kind::Absent)])).unwrap();
        assert_eq!(m.entries.len(), 2);
        assert!(
            matches(&m, &t).is_err(),
            "an impossible log must never match"
        );
    }

    #[test]
    fn record_children_adds_picked_files_of_listed_dirs_only() {
        let t = Tree::from_leaves([
            Entry::file("pkg/a.go", Hash::of(b"a"), 0o644),
            Entry::file("pkg/a_windows.go", Hash::of(b"w"), 0o644),
            Entry::file("pkg/README.md", Hash::of(b"r"), 0o644),
            Entry::file("pkg/sub/b.go", Hash::of(b"b"), 0o644),
            Entry::file("other/c.go", Hash::of(b"c"), 0o644),
            Entry::symlink("pkg/link.go", "a.go"),
        ]);
        let mut l = log(&[("pkg", Kind::Dir), ("pkg/a.go", Kind::File)]);
        record_children(&t, &mut l, |n| n.ends_with(".go"));
        let mut got: Vec<&str> = l.iter().map(|a| a.path.as_str()).collect();
        got.sort();
        // Listed dir's .go files, no recursion, no symlinks, no other dirs.
        assert_eq!(got, ["pkg", "pkg/a.go", "pkg/a.go", "pkg/a_windows.go"]);
        let m = Manifest::from_log(Hash::of(b"k"), vec![], &t, l.clone()).unwrap();
        assert_eq!(m.entries.len(), 3);
        let before = l.len();
        record_children(&t, &mut l, |n| n.ends_with(".go"));
        assert_eq!(
            Manifest::from_log(Hash::of(b"k"), vec![], &t, l.clone())
                .unwrap()
                .entries
                .len(),
            3,
            "idempotent after dedup"
        );
        assert_eq!(l.len(), before + 2);
    }

    #[test]
    fn schema_field_names() {
        let t = tree();
        let m = Manifest::from_log(
            Hash::of(b"k"),
            vec!["/workspace".into()],
            &t,
            log(&[
                ("go.mod", Kind::File),
                ("bin/gh", Kind::Symlink),
                ("gone", Kind::Absent),
            ]),
        )
        .unwrap();
        let j = m.canonical_json().unwrap();
        assert!(
            j.contains(r#"{"k":"l","p":"bin/gh","t":"../build/gh"}"#),
            "{j}"
        );
        assert!(j.contains(r#"{"k":"a","p":"gone"}"#), "{j}");
        assert!(j.contains(r#""k":"f","m":420,"p":"go.mod"}"#), "{j}");
        assert!(j.ends_with(r#""v":2}"#));
        let back: Manifest = serde_json::from_str(&j).unwrap();
        assert_eq!(back, m);
        assert_eq!(manifest_id(&back).unwrap(), manifest_id(&m).unwrap());

        // A result without root outputs serializes without the field, and a
        // stored one without it reads back as empty.
        let mut r = StepResult {
            v: VERSION,
            step_key: Hash::of(b"k"),
            manifest_id: manifest_id(&m).unwrap(),
            run_id: "01RUN".into(),
            recorded_at: "2026-10-31T00:00:00.000Z".into(),
            exit_code: 0,
            stdout: Hash::of(b""),
            stderr: Hash::of(b""),
            outputs: vec![Entry::file("out.txt", Hash::of(b"w"), 0o644)],
            root_outputs: vec![],
            duration_ms: 1,
        };
        let j = canonical_json(&r).unwrap();
        assert!(!j.contains("root_outputs"), "{j}");
        assert_eq!(serde_json::from_str::<StepResult>(&j).unwrap(), r);
        r.root_outputs = vec![Entry::file("opt/marker", Hash::of(b"m"), 0o644)];
        let j = canonical_json(&r).unwrap();
        assert!(
            j.contains(r#""root_outputs":[{"h":"b3:"#) && j.contains(r#""p":"opt/marker"}]"#),
            "{j}"
        );
        assert_eq!(serde_json::from_str::<StepResult>(&j).unwrap(), r);
    }

    #[test]
    fn mismatch_messages() {
        let t = tree();
        let m = Manifest::from_log(
            Hash::of(b"k"),
            vec![],
            &t,
            log(&[("pkg/cmd", Kind::Dir), ("pkg/cmd/root.go", Kind::File)]),
        )
        .unwrap();
        let t2 = t.with_outputs(&[Entry::file("pkg/cmd/root.go", Hash::of(b"changed"), 0o644)]);
        let e = matches(&m, &t2).unwrap_err();
        assert_eq!(e.path, "pkg/cmd/root.go");
        assert!(
            e.to_string()
                .starts_with("pkg/cmd/root.go: file content changed"),
            "{e}"
        );
        let t3 = t.with_outputs(&[Entry::file("pkg/cmd/new.go", Hash::of(b"n"), 0o644)]);
        let e = matches(&m, &t3).unwrap_err();
        assert_eq!(e.path, "pkg/cmd");
        assert!(
            e.to_string()
                .starts_with("pkg/cmd: directory listing changed"),
            "{e}"
        );
    }
}
