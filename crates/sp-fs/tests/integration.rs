//! FUSE integration tests. Need Linux, /dev/fuse and root or user_allow_other;
//! gated on STILLPASS_INTEGRATION=1.
#![cfg(target_os = "linux")]

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

use sp_cas::{Cas, Kind, Tree};
use sp_manifest::Access;

fn enabled() -> bool {
    let on = std::env::var("STILLPASS_INTEGRATION").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("STILLPASS_INTEGRATION != 1; skipping");
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

fn mounted(tree: &Tree, cas: &Cas, tmp: &Path) -> sp_fs::Served {
    let mnt = tmp.join("mnt");
    fs::create_dir_all(&mnt).unwrap();
    sp_fs::serve(tree, cas, &mnt).unwrap()
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
    let served = mounted(&tree, &cas, tmp.path());
    // Settle the root first (the mount itself stats it), then measure.
    sh(served.mountpoint(), "true");
    let _ = served.drain();
    let out = sh(served.mountpoint(), "ls pkg/cmd");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "big.bin\nroot.go\n");
    let log = served.drain();
    assert_eq!(kinds_for(&log, "pkg/cmd"), [Kind::Dir]);
    assert!(
        kinds_for(&log, "pkg/cmd/root.go").is_empty(),
        "plain ls must not read children: {log:?}"
    );

    sh(
        served.mountpoint(),
        "stat pkg/nope.go >/dev/null 2>&1 || true",
    );
    let log = served.drain();
    assert_eq!(kinds_for(&log, "pkg/nope.go"), [Kind::Absent]);
    assert_eq!(
        log.iter().filter(|a| a.kind == Kind::Absent).count(),
        1,
        "{log:?}"
    );

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

/// T05 definition of done on the spike repo, when STILLPASS_SPIKE_REPO is set.
#[test]
fn spike_repo_read_set_equals_file_set() {
    if !enabled() {
        return;
    }
    let Ok(repo) = std::env::var("STILLPASS_SPIKE_REPO") else {
        eprintln!("STILLPASS_SPIKE_REPO not set; skipping");
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
