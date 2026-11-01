use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;

use proptest::prelude::*;
use quod_cas::{Cas, Kind};

#[derive(Debug, Clone)]
enum Node {
    File(Vec<u8>, u32),
    Link(String),
    Dir(Vec<(String, Node)>),
}

fn arb_node() -> impl Strategy<Value = Node> {
    let leaf = prop_oneof![
        (
            prop::collection::vec(any::<u8>(), 0..64),
            prop::sample::select(vec![0o644u32, 0o755, 0o600, 0o444])
        )
            .prop_map(|(b, m)| Node::File(b, m)),
        "[a-z]{1,6}(/[a-z]{1,6})?".prop_map(Node::Link),
    ];
    leaf.prop_recursive(3, 24, 5, |inner| {
        prop::collection::btree_map("[a-z0-9_-][a-z0-9._-]{0,7}", inner, 0..5)
            .prop_map(|m| Node::Dir(m.into_iter().collect()))
    })
}

fn write(dir: &Path, kids: &[(String, Node)]) {
    for (name, node) in kids {
        let p = dir.join(name);
        match node {
            Node::File(b, m) => {
                fs::write(&p, b).unwrap();
                fs::set_permissions(&p, fs::Permissions::from_mode(*m)).unwrap();
            }
            Node::Link(t) => symlink(t, &p).unwrap(),
            Node::Dir(sub) => {
                fs::create_dir(&p).unwrap();
                write(&p, sub);
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn import_then_materialize_is_identity(
        root in prop::collection::btree_map("[a-z0-9_-][a-z0-9._-]{0,7}", arb_node(), 0..6)
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir(&src).unwrap();
        write(&src, &root.into_iter().collect::<Vec<_>>());

        let cas = Cas::open(tmp.path().join("cas")).unwrap();
        let tree = cas.import_dir(&src).unwrap();
        let out = tmp.path().join("out");
        cas.materialize(&tree, &out).unwrap();
        let again = cas.import_dir(&out).unwrap();
        prop_assert_eq!(&tree, &again);
        for e in tree.iter().filter(|e| e.k == Kind::File) {
            prop_assert!(cas.has(&e.h.unwrap()));
        }
    }
}

#[test]
fn import_is_stable_and_dedups() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join("a/b")).unwrap();
    fs::write(src.join("a/b/one"), b"same").unwrap();
    fs::write(src.join("two"), b"same").unwrap();
    let big = vec![7u8; (1 << 20) + 17];
    fs::write(src.join("big"), &big).unwrap();
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let (t1, s1) = cas.import_dir_stats(&src).unwrap();
    let (t2, s2) = cas.import_dir_stats(&src).unwrap();
    assert_eq!(t1, t2);
    assert_eq!((s1.files, s1.bytes, s1.dirs), (3, 8 + big.len() as u64, 3));
    // Two workers may both store the duplicate before either sees it.
    assert!((4 + big.len() as u64..=8 + big.len() as u64).contains(&s1.new_bytes));
    assert_eq!(s2.new_bytes, 0);
    assert_eq!(
        cas.get(&t1.get("big").unwrap().h.unwrap())
            .unwrap()
            .unwrap(),
        big
    );
}

#[test]
fn take_file_moves_new_blobs_and_leaves_duplicates() {
    let tmp = tempfile::tempdir().unwrap();
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let out = tmp.path().join("out");
    fs::create_dir(&out).unwrap();
    fs::write(out.join("fresh"), b"fresh bytes").unwrap();
    fs::set_permissions(out.join("fresh"), fs::Permissions::from_mode(0o600)).unwrap();
    let big = vec![9u8; (1 << 20) + 3];
    fs::write(out.join("big"), &big).unwrap();
    let known = cas.put(b"already here").unwrap();
    fs::write(out.join("dup"), b"already here").unwrap();

    let mut synced = false;
    let got = cas
        .take_files(
            &[out.join("fresh"), out.join("big"), out.join("dup")],
            |d| {
                // Stands in for syncfs: the store asks for the flush before any
                // new file is renamed to its blob name.
                assert!(d == cas.dir() && out.join("fresh").exists());
                synced = true;
                fs::File::open(d)?.sync_all()
            },
        )
        .unwrap();
    assert!(synced);
    assert_eq!(got[0], (quod_cas::Hash::of(b"fresh bytes"), 11));
    assert_eq!(got[1].1, big.len() as u64);
    assert_eq!(got[2], (known, 0));
    // New blobs moved; the duplicate stayed; the store reads back exactly.
    assert!(!out.join("fresh").exists() && !out.join("big").exists());
    assert!(out.join("dup").exists());
    assert_eq!(cas.get(&got[0].0).unwrap().unwrap(), b"fresh bytes");
    assert_eq!(cas.get(&got[1].0).unwrap().unwrap(), big);
    // Moved (linked, then unlinked from the output dir): one name left.
    for g in &got[..2] {
        assert_eq!(fs::metadata(cas.blob_path(&g.0)).unwrap().nlink(), 1);
    }
    // Whatever mode the step used, the blob has the store's: read-only.
    let mode = fs::metadata(cas.blob_path(&got[0].0)).unwrap().mode() & 0o777;
    assert_eq!(mode, 0o444);
    // put_file of the same content agrees with the moved blob's hash.
    fs::write(out.join("again"), b"fresh bytes").unwrap();
    assert_eq!(cas.put_file(&out.join("again")).unwrap(), (got[0].0, 0));

    // A hard-linked file is copied, not moved: its other name could still
    // write to it. Two new files with the same content move only once.
    fs::write(out.join("linked"), b"linked bytes").unwrap();
    fs::hard_link(out.join("linked"), out.join("alias")).unwrap();
    fs::write(out.join("twin1"), b"twins").unwrap();
    fs::write(out.join("twin2"), b"twins").unwrap();
    let got = cas
        .take_files(
            &[out.join("linked"), out.join("twin1"), out.join("twin2")],
            |_| Ok(()),
        )
        .unwrap();
    assert!(out.join("linked").exists() && out.join("alias").exists());
    assert_eq!(cas.get(&got[0].0).unwrap().unwrap(), b"linked bytes");
    assert_eq!(got[1].0, got[2].0);
    assert_eq!(cas.get(&got[1].0).unwrap().unwrap(), b"twins");
    assert!(out.join("twin1").exists() != out.join("twin2").exists());
    assert_eq!(
        fs::metadata(cas.blob_path(&got[1].0)).unwrap().nlink(),
        1,
        "the blob has one name"
    );
}

/// T02 definition of done: importing the spike repo completes and reports
/// file count and bytes. Runs when QUOD_SPIKE_REPO points at a checkout.
#[test]
fn import_spike_repo() {
    let Ok(repo) = std::env::var("QUOD_SPIKE_REPO") else {
        eprintln!("QUOD_SPIKE_REPO not set; skipping");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let start = std::time::Instant::now();
    let (tree, s) = cas.import_dir_stats(Path::new(&repo)).unwrap();
    eprintln!(
        "imported {repo}: {} files, {} bytes, {} dirs, {} symlinks, {} entries in {:?}",
        s.files,
        s.bytes,
        s.dirs,
        s.symlinks,
        tree.len(),
        start.elapsed()
    );
    assert!(s.files > 0);
}

#[test]
fn link_entries_is_a_snapshot_of_the_hashed_content() {
    use std::os::unix::fs::MetadataExt;
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join("m@v1/sub")).unwrap();
    fs::write(src.join("m@v1/a.go"), b"package m\n").unwrap();
    fs::set_permissions(src.join("m@v1/a.go"), fs::Permissions::from_mode(0o444)).unwrap();
    // Same content, different mode: cannot share an inode with a.go.
    fs::write(src.join("m@v1/sub/run.sh"), b"package m\n").unwrap();
    fs::set_permissions(
        src.join("m@v1/sub/run.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    symlink("a.go", src.join("m@v1/l")).unwrap();
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas.import_dir(&src).unwrap();

    let snap = tmp.path().join("snap");
    cas.link_entries(tree.iter(), &snap).unwrap();
    assert_eq!(
        cas.import_dir(&snap).unwrap(),
        tree,
        "the snapshot is the tree"
    );
    let a = fs::metadata(snap.join("m@v1/a.go")).unwrap();
    let blob = fs::metadata(cas.blob_path(&tree.get("m@v1/a.go").unwrap().h.unwrap())).unwrap();
    assert_eq!(a.ino(), blob.ino(), "a.go is a hard link to its blob");
    assert_eq!(a.mode() & 0o777, 0o444);
    let sh = fs::metadata(snap.join("m@v1/sub/run.sh")).unwrap();
    assert_ne!(
        sh.ino(),
        blob.ino(),
        "a second mode for the same content is a copy"
    );
    assert_eq!(sh.mode() & 0o777, 0o755);
    assert_eq!(
        fs::read_link(snap.join("m@v1/l")).unwrap().to_str(),
        Some("a.go")
    );

    // Linking never changes a blob's mode: a second snapshot wanting another
    // mode for the same content copies instead (see link_entries).
    assert_eq!(blob.mode() & 0o777, 0o444);
    assert_eq!(
        fs::metadata(cas.blob_path(&tree.get("m@v1/a.go").unwrap().h.unwrap()))
            .unwrap()
            .mode()
            & 0o777,
        0o444
    );
    // Changing the source afterwards does not reach the snapshot.
    fs::set_permissions(src.join("m@v1/a.go"), fs::Permissions::from_mode(0o644)).unwrap();
    fs::write(src.join("m@v1/a.go"), b"package changed\n").unwrap();
    assert_eq!(fs::read(snap.join("m@v1/a.go")).unwrap(), b"package m\n");
}

#[test]
fn import_dir_except_is_import_dir_without_those_top_level_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let src = tmp.path().join("src");
    for p in [
        ".git/objects/ab/cd",
        ".git/HEAD",
        "a/.git",
        "b.txt",
        ".quod/x/y",
        "gitx",
    ] {
        let p = src.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, p.to_string_lossy().as_bytes()).unwrap();
    }
    // A large file (streamed) whose blob the store already has.
    let big = vec![7u8; (2 << 20) + 1];
    fs::write(src.join("big"), &big).unwrap();
    let known = cas.put(&big).unwrap();
    let whole = cas.import_dir(&src).unwrap();
    let cas2 = Cas::open(tmp.path().join("cas2")).unwrap();
    let except = cas2.import_dir_except(&src, &[".git", ".quod"]).unwrap();
    assert_eq!(
        except,
        whole.without_top_level(".git").without_top_level(".quod")
    );
    // Not stored: no blob of the skipped files in the second store.
    let head = quod_cas::Hash::of(src.join(".git/HEAD").to_string_lossy().as_bytes());
    assert!(!cas2.has(&head) && cas.has(&head));
    // A nested .git is an ordinary entry.
    assert!(except.get("a/.git").is_some());
    assert_eq!(cas.put_file(&src.join("big")).unwrap(), (known, 0));
}
