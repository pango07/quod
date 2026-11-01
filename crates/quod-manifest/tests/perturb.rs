//! Property tests: identical trees match; any single perturbation of a
//! manifest entry mismatches and names the path; changes elsewhere still
//! match.

use proptest::prelude::*;
use quod_manifest::{Access, Entry, Hash, Kind, Manifest, Tree, matches};

#[derive(Debug, Clone)]
struct Case {
    tree: Tree,
    manifest: Manifest,
}

fn arb_case() -> impl Strategy<Value = Case> {
    let leaf = (
        "[a-c]{1,2}(/[a-c]{1,2}){0,3}",
        prop_oneof![
            (
                any::<[u8; 4]>(),
                prop::sample::select(vec![0o644u32, 0o755])
            )
                .prop_map(|(b, m)| (Some((b, m)), None)),
            "[a-z]{1,4}".prop_map(|t| (None, Some(t))),
        ],
    );
    (
        prop::collection::vec(leaf, 1..20),
        prop::collection::vec(any::<prop::sample::Index>(), 1..30),
        prop::collection::vec("[d-f]{1,3}", 0..4),
    )
        .prop_filter_map(
            "leaf paths collide with directories",
            |(leaves, picks, absent)| {
                let mut entries: Vec<Entry> = Vec::new();
                for (p, (file, link)) in leaves {
                    if entries.iter().any(|e| {
                        e.p == p
                            || e.p.starts_with(&format!("{p}/"))
                            || p.starts_with(&format!("{}/", e.p))
                    }) {
                        continue;
                    }
                    entries.push(match (file, link) {
                        (Some((b, m)), _) => Entry::file(p, Hash::of(&b), m),
                        (_, Some(t)) => Entry::symlink(p, t),
                        _ => unreachable!(),
                    });
                }
                if entries.is_empty() {
                    return None;
                }
                let tree = Tree::from_leaves(entries);
                let all: Vec<&Entry> = tree.iter().collect();
                let mut log: Vec<Access> = picks
                    .iter()
                    .map(|i| {
                        let e = i.get(&all);
                        Access {
                            path: e.p.clone(),
                            kind: e.k,
                        }
                    })
                    .collect();
                log.extend(absent.into_iter().map(|p| Access {
                    path: format!("zz/{p}"),
                    kind: Kind::Absent,
                }));
                let manifest =
                    Manifest::from_log(Hash::of(b"key"), vec!["/workspace".into()], &tree, log)
                        .ok()?;
                Some(Case { tree, manifest })
            },
        )
}

/// `got` is `path` or one of its ancestor directories (whose listing a
/// created or removed path also changes, and which sorts first).
fn on_path(got: &str, path: &str) -> bool {
    got == path || got == "." || path.starts_with(&format!("{got}/"))
}

fn leaves(t: &Tree) -> Vec<Entry> {
    t.iter().filter(|e| e.k != Kind::Dir).cloned().collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn identical_tree_matches(c in arb_case()) {
        prop_assert!(matches(&c.manifest, &c.tree).is_ok());
    }

    #[test]
    fn content_or_mode_change_mismatches(c in arb_case(), pick in any::<prop::sample::Index>(), mode_only in any::<bool>()) {
        let files: Vec<&Entry> = c.manifest.entries.iter().filter(|e| e.k == Kind::File).collect();
        prop_assume!(!files.is_empty());
        let target = pick.get(&files);
        let changed = if mode_only {
            Entry::file(target.p.clone(), target.h.unwrap(), target.m.unwrap() ^ 0o100)
        } else {
            Entry::file(target.p.clone(), Hash::of(b"different bytes"), target.m.unwrap())
        };
        let t = c.tree.with_outputs(&[changed]);
        let e = matches(&c.manifest, &t).unwrap_err();
        prop_assert_eq!(&e.path, &target.p);
    }

    #[test]
    fn symlink_target_change_mismatches(c in arb_case(), pick in any::<prop::sample::Index>()) {
        let links: Vec<&Entry> = c.manifest.entries.iter().filter(|e| e.k == Kind::Symlink).collect();
        prop_assume!(!links.is_empty());
        let target = pick.get(&links);
        let t = c.tree.with_outputs(&[Entry::symlink(target.p.clone(), "elsewhere/x")]);
        prop_assert_eq!(matches(&c.manifest, &t).unwrap_err().path, target.p.clone());
    }

    #[test]
    fn listing_change_mismatches(c in arb_case(), pick in any::<prop::sample::Index>()) {
        let dirs: Vec<&Entry> = c.manifest.entries.iter().filter(|e| e.k == Kind::Dir).collect();
        prop_assume!(!dirs.is_empty());
        let target = pick.get(&dirs);
        let new = quod_cas::child_path(&target.p, "~new");
        let t = c.tree.with_outputs(&[Entry::file(new, Hash::of(b"n"), 0o644)]);
        prop_assert_eq!(matches(&c.manifest, &t).unwrap_err().path, target.p.clone());
    }

    #[test]
    fn absent_now_present_mismatches(c in arb_case(), pick in any::<prop::sample::Index>()) {
        let absent: Vec<&Entry> = c.manifest.entries.iter().filter(|e| e.k == Kind::Absent).collect();
        prop_assume!(!absent.is_empty());
        let target = pick.get(&absent);
        let t = c.tree.with_outputs(&[Entry::file(target.p.clone(), Hash::of(b"appeared"), 0o644)]);
        let e = matches(&c.manifest, &t).unwrap_err();
        prop_assert!(on_path(&e.path, &target.p), "mismatch at {} for created {}", e.path, target.p);
    }

    #[test]
    fn present_now_absent_mismatches(c in arb_case(), pick in any::<prop::sample::Index>()) {
        let present: Vec<&Entry> = c.manifest.entries.iter()
            .filter(|e| matches!(e.k, Kind::File | Kind::Symlink)).collect();
        prop_assume!(!present.is_empty());
        let target = pick.get(&present);
        let t = Tree::from_leaves(leaves(&c.tree).into_iter().filter(|e| e.p != target.p));
        let e = matches(&c.manifest, &t).unwrap_err();
        // The parent's listing may be recorded too and sorts first; either
        // way the mismatch must be on the path or one of its ancestors.
        prop_assert!(on_path(&e.path, &target.p), "mismatch at {} for removed {}", e.path, target.p);
    }

    #[test]
    fn changes_outside_the_manifest_still_match(c in arb_case()) {
        let recorded: std::collections::BTreeSet<&str> = c.manifest.entries.iter().map(|e| e.p.as_str()).collect();
        // Change every file the manifest doesn't mention whose parent listing
        // isn't recorded either (content changes leave listings alone).
        let changed: Vec<Entry> = c.tree.iter()
            .filter(|e| e.k == Kind::File && !recorded.contains(e.p.as_str()))
            .map(|e| Entry::file(e.p.clone(), Hash::of(b"other"), e.m.unwrap()))
            .collect();
        let t = c.tree.with_outputs(&changed);
        prop_assert!(matches(&c.manifest, &t).is_ok());
    }
}
