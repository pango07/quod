//! FUSE integration tests. Need Linux, /dev/fuse and root or user_allow_other;
//! gated on QUOD_INTEGRATION=1.
#![cfg(target_os = "linux")]

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

use quod_cas::{Cas, Kind, Tree};
use quod_manifest::Access;

fn enabled() -> bool {
    let on = std::env::var("QUOD_INTEGRATION").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("QUOD_INTEGRATION != 1; skipping");
    }
    on
}

fn sample(dir: &Path) {
    fs::create_dir_all(dir.join("pkg/cmd")).unwrap();
    fs::create_dir_all(dir.join("empty")).unwrap();
    fs::write(dir.join("go.mod"), "module x\n").unwrap();
    fs::write(dir.join("pkg/cmd/root.go"), "package cmd\n").unwrap();
    fs::write(dir.join("pkg/cmd/big.bin"), vec![42u8; 3 << 20]).unwrap();
    symlink("pkg/cmd/root.go", dir.join("link")).unwrap();
}

fn kinds_for(log: &[Access], path: &str) -> Vec<Kind> {
    log.iter()
        .filter(|a| a.path == path)
        .map(|a| a.kind)
        .collect()
}

fn sh(cwd: &Path, script: &str) -> std::process::Output {
    let out = Command::new("sh")
        .arg("-c")
        .arg(script)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.status.success() || script.contains("|| true"),
        "{script}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn mounted(tree: &Tree, cas: &Cas, tmp: &Path) -> quod_fs::Served {
    mounted_at(tree, cas, &tmp.join("mnt"))
}

fn mounted_at(tree: &Tree, cas: &Cas, mnt: &Path) -> quod_fs::Served {
    fs::create_dir_all(mnt).unwrap();
    quod_fs::serve(tree, cas, mnt).unwrap()
}

#[test]
fn cat_every_file_logs_exactly_the_file_set() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    sample(&tmp.path().join("src"));
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas.import_dir(&tmp.path().join("src")).unwrap();
    let served = mounted(&tree, &cas, tmp.path());
    let out = sh(served.mountpoint(), "find . -type f -exec cat {} + | wc -c");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        ((3 << 20) + 9 + 12).to_string()
    );
    assert_eq!(
        fs::read_to_string(served.mountpoint().join("link")).unwrap(),
        "package cmd\n"
    );
    // By default opens are zero-message: the first OPEN is answered ENOSYS
    // and the kernel sends no other, reading through its page cache. With
    // kernel passthrough (Linux 6.9+, CAP_SYS_ADMIN, QUOD_FUSE_ZERO_OPEN=0)
    // no read reaches userspace at all. The log below must be complete
    // either way.
    let stats = served.stats();
    eprintln!("observer stats: {stats:?}");
    if stats.zero_open {
        assert_eq!(stats.open, 1, "{stats:?}");
        assert!(!stats.passthrough, "{stats:?}");
    } else {
        assert!(stats.open >= 4, "{stats:?}");
    }
    if stats.passthrough {
        assert_eq!(stats.read, 0, "passthrough reads must not reach userspace");
    } else {
        assert!(stats.read_bytes >= 3 << 20, "{stats:?}");
    }
    let log = served.unmount().unwrap();
    let logged: BTreeSet<&str> = log
        .iter()
        .filter(|a| a.kind == Kind::File)
        .map(|a| a.path.as_str())
        .collect();
    let files: BTreeSet<&str> = tree
        .iter()
        .filter(|e| e.k == Kind::File)
        .map(|e| e.p.as_str())
        .collect();
    assert_eq!(logged, files);
    assert_eq!(kinds_for(&log, "link"), [Kind::Symlink]);
}

#[test]
fn ls_logs_one_listing_and_stat_of_missing_logs_one_absent() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    sample(&tmp.path().join("src"));
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas.import_dir(&tmp.path().join("src")).unwrap();
    // One mount per check: a mount serves one step, and kernel caching
    // means a second command on the same mount would not be logged afresh.
    let served = mounted_at(&tree, &cas, &tmp.path().join("m1"));
    let out = sh(served.mountpoint(), "ls pkg/cmd");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "big.bin\nroot.go\n");
    let log = served.unmount().unwrap();
    assert_eq!(kinds_for(&log, "pkg/cmd"), [Kind::Dir]);
    assert!(
        kinds_for(&log, "pkg/cmd/root.go").is_empty(),
        "plain ls must not read children: {log:?}"
    );

    let served = mounted_at(&tree, &cas, &tmp.path().join("m2"));
    sh(
        served.mountpoint(),
        "stat pkg/nope.go >/dev/null 2>&1 || true",
    );
    let log = served.unmount().unwrap();
    assert_eq!(kinds_for(&log, "pkg/nope.go"), [Kind::Absent]);
    assert_eq!(
        log.iter().filter(|a| a.kind == Kind::Absent).count(),
        1,
        "{log:?}"
    );

    let served = mounted_at(&tree, &cas, &tmp.path().join("m3"));
    // Writes are refused: this mount is read-only.
    assert!(fs::write(served.mountpoint().join("go.mod"), "x").is_err());
    let mnt = served.mountpoint().to_path_buf();
    served.unmount().unwrap();
    let mounts = fs::read_to_string("/proc/self/mounts").unwrap();
    assert!(
        !mounts.contains(mnt.to_str().unwrap()),
        "unmount left the mount behind"
    );
}

/// T05 definition of done on the spike repo, when QUOD_SPIKE_REPO is set.
#[test]
fn spike_repo_read_set_equals_file_set() {
    if !enabled() {
        return;
    }
    let Ok(repo) = std::env::var("QUOD_SPIKE_REPO") else {
        eprintln!("QUOD_SPIKE_REPO not set; skipping");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas
        .import_dir(Path::new(&repo))
        .unwrap()
        .without_top_level(".git");
    let served = mounted(&tree, &cas, tmp.path());
    sh(
        served.mountpoint(),
        "find . -type f -exec cat {} + >/dev/null",
    );
    let log = served.unmount().unwrap();
    let logged: BTreeSet<&str> = log
        .iter()
        .filter(|a| a.kind == Kind::File)
        .map(|a| a.path.as_str())
        .collect();
    let files: BTreeSet<&str> = tree
        .iter()
        .filter(|e| e.k == Kind::File)
        .map(|e| e.p.as_str())
        .collect();
    eprintln!(
        "spike repo: {} files served, {} logged as read, {} log entries",
        files.len(),
        logged.len(),
        log.len()
    );
    assert_eq!(logged, files);
}

#[test]
fn overlay_diff_reports_exactly_the_writes() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    sample(&tmp.path().join("src"));
    fs::write(tmp.path().join("src/pkg/old.txt"), "old").unwrap();
    fs::create_dir_all(tmp.path().join("src/gone/sub")).unwrap();
    fs::write(tmp.path().join("src/gone/sub/f"), "f").unwrap();
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas.import_dir(&tmp.path().join("src")).unwrap();
    let served = mounted(&tree, &cas, tmp.path());
    let ov = quod_fs::overlay(
        &served,
        &tmp.path().join("upper"),
        &tmp.path().join("work"),
        &tmp.path().join("merged"),
    )
    .unwrap();
    sh(
        ov.merged(),
        "echo new > created.txt && mkdir -p newdir/deep && echo d > newdir/deep/x \
         && echo more >> go.mod && rm pkg/cmd/big.bin && mv pkg/old.txt pkg/renamed.txt \
         && rm -r gone && chmod 755 pkg/cmd/root.go",
    );
    let out = ov.finish(&tree, &cas).unwrap();
    let log = served.unmount().unwrap();
    let got: Vec<(String, Kind)> = out.iter().map(|e| (e.p.clone(), e.k)).collect();
    let want: Vec<(String, Kind)> = [
        ("created.txt", Kind::File),
        ("gone", Kind::Deleted),
        ("go.mod", Kind::File),
        ("newdir", Kind::Dir),
        ("newdir/deep", Kind::Dir),
        ("newdir/deep/x", Kind::File),
        ("pkg/cmd/big.bin", Kind::Deleted),
        ("pkg/cmd/root.go", Kind::File),
        ("pkg/old.txt", Kind::Deleted),
        ("pkg/renamed.txt", Kind::File),
    ]
    .iter()
    .map(|(p, k)| (p.to_string(), *k))
    .collect();
    let mut got_sorted = got.clone();
    got_sorted.sort();
    let mut want_sorted = want.clone();
    want_sorted.sort();
    assert_eq!(got_sorted, want_sorted);
    let gomod = out.iter().find(|e| e.p == "go.mod").unwrap();
    assert_eq!(
        cas.get(&gomod.h.unwrap()).unwrap().unwrap(),
        b"module x\nmore\n"
    );
    assert_eq!(
        out.iter().find(|e| e.p == "pkg/cmd/root.go").unwrap().m,
        Some(0o755)
    );

    // Writes reach only the upper dir: the lower log never shows a path the
    // step created as present, only as absent lookups.
    for created in ["created.txt", "newdir", "newdir/deep/x", "pkg/renamed.txt"] {
        assert!(
            kinds_for(&log, created).iter().all(|k| *k == Kind::Absent),
            "{created}: {log:?}"
        );
    }
    assert!(fs::read_to_string(tmp.path().join("src/go.mod")).unwrap() == "module x\n");

    // Applying the outputs to the tree reproduces the merged view.
    let after = tree.with_outputs(&out);
    assert!(after.get("gone").is_none() && after.get("gone/sub/f").is_none());
    assert!(after.get("newdir/deep/x").is_some() && after.get("pkg/old.txt").is_none());
}

/// Sets an xattr, as a step may on its own files.
fn set_xattr(path: &Path, name: &str, value: &[u8]) {
    use std::os::unix::ffi::OsStrExt;
    let p = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    let n = std::ffi::CString::new(name).unwrap();
    // SAFETY: valid C strings and a buffer valid for its length.
    #[allow(unsafe_code)] // setxattr(2) has no safe std wrapper
    let rc = unsafe {
        libc::setxattr(
            p.as_ptr(),
            n.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    assert_eq!(
        rc,
        0,
        "setxattr {name} on {}: {}",
        path.display(),
        std::io::Error::last_os_error()
    );
}

/// The overlay reads its markers in `trusted.overlay.*` only (no
/// userxattr), so a `user.overlay.opaque` a step sets on a directory is an
/// ordinary xattr: the merged view still shows the lower contents, and the
/// diff must not claim they were deleted (that output would replay as a
/// deletion a fresh run never makes).
#[test]
fn a_steps_user_overlay_xattrs_mean_nothing() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    sample(&tmp.path().join("src"));
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas.import_dir(&tmp.path().join("src")).unwrap();
    let served = mounted(&tree, &cas, tmp.path());
    let ov = quod_fs::overlay(
        &served,
        &tmp.path().join("upper"),
        &tmp.path().join("work"),
        &tmp.path().join("merged"),
    )
    .unwrap();
    set_xattr(&ov.merged().join("pkg"), "user.overlay.opaque", b"y");
    fs::create_dir(ov.merged().join("mine")).unwrap();
    set_xattr(&ov.merged().join("mine"), "user.overlay.redirect", b"/pkg");
    assert!(ov.merged().join("pkg/cmd/root.go").exists(), "still merged");
    let out = ov.finish(&tree, &cas).unwrap();
    served.unmount().unwrap();
    let got: Vec<(String, Kind)> = out.iter().map(|e| (e.p.clone(), e.k)).collect();
    assert_eq!(got, vec![("mine".to_string(), Kind::Dir)], "{got:?}");
}

/// Zero-message opens and opendirs only let the kernel skip requests that
/// would record nothing new (`CasFs::record`): on the classic transport
/// (what CI has), the same workloads log exactly the same accesses, and
/// show the step the same bytes, with every open and opendir answered
/// (with and without passthrough) as with the defaults. Each workload
/// repeats its reads and listings, which the default mode serves from the
/// kernel's caches. quod-cli's transports test compares the ring too.
#[test]
fn zero_message_opens_log_what_answered_opens_log() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    sample(&src);
    for i in 0..300 {
        fs::write(src.join(format!("pkg/f{i:03}.txt")), format!("{i}\n")).unwrap();
    }
    let cas = Cas::open(tmp.path().join("cas")).unwrap();
    let tree = cas.import_dir(&src).unwrap();
    let workloads = [
        "for i in 1 2 3; do find . -type f -exec cat {} + | cksum; done",
        "for i in 1 2 3; do ls -lA . pkg pkg/cmd empty; ls -f pkg | sort | cksum; done",
        "stat pkg/nope.go nope/x 2>&1; stat pkg/nope.go 2>&1; cat link; readlink link; true",
        "dd if=pkg/cmd/big.bin bs=4096 skip=300 count=3 2>/dev/null | cksum; head -c 7 go.mod",
    ];
    let mut n = 0;
    for script in workloads {
        let mut seen: Vec<(String, quod_fs::Observed, quod_fs::ObserverStats)> = Vec::new();
        for (zero, passthrough) in [(true, false), (false, true), (false, false)] {
            n += 1;
            let mnt = tmp.path().join(format!("m{n}"));
            fs::create_dir_all(&mnt).unwrap();
            let opts = quod_fs::Options {
                transport: quod_fs::Transport::Classic,
                passthrough,
                zero_open: zero,
                zero_opendir: zero,
                xattr_enosys: zero,
                ..quod_fs::Options::default()
            };
            let served = quod_fs::serve_with(&tree, &cas, &mnt, &opts).unwrap();
            let out = sh(&mnt, script);
            let stats = served.stats();
            let log = served.unmount_observed().unwrap();
            assert!(!log.accesses.is_empty(), "{script}: nothing logged");
            seen.push((String::from_utf8(out.stdout).unwrap(), log, stats));
        }
        let (out, log, zs) = &seen[0];
        assert!(zs.zero_open && zs.zero_opendir, "{zs:?}");
        for (o, l, s) in &seen[1..] {
            assert_eq!(o, out, "{script}: the step saw something else ({s:?})");
            assert_eq!(
                l.accesses, log.accesses,
                "{script}: answered opens logged differently ({s:?})"
            );
            // These workloads read every file they open, so the files
            // served bytes are the same set either way (with zero-message
            // opens it comes from the READs; see `Options::zero_open`).
            assert_eq!(l.opened, log.opened, "{script}: opened ({s:?})");
        }
        if script.contains("find . -type f") {
            // 300-odd files read three times: one OPEN reached the observer.
            assert_eq!(zs.open, 1, "{zs:?}");
            assert!(seen[2].2.open >= 3 * 300, "{:?}", seen[2].2);
        }
        if script.contains("ls -f pkg") {
            // The listings are read once, then from the kernel's cache.
            assert!(zs.readdir < seen[2].2.readdir, "{zs:?} {:?}", seen[2].2);
        }
    }
}
