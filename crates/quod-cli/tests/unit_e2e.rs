//! `quod unit` end to end, with the real strace and bubblewrap: the first
//! run is traced and recorded, the second (plain) agrees, the third is
//! answered from the record (its output printed, its files restored), and
//! a changed input runs it again. Skipped where strace or an unprivileged
//! bubblewrap is missing (asserted present on CI). Runs as the runner's
//! user and as root (the lint gate): each step has a deadline, so a hang
//! fails with what the step printed instead of holding the gate.

use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn usable(prog: &str, args: &[&str]) -> bool {
    Command::new(prog)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn unit(dir: &Path, root: &Path, script: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_quod"))
        .current_dir(root)
        .arg("unit")
        .arg("--dir")
        .arg(dir)
        .arg("--root")
        .arg(root)
        .arg("--mark")
        .arg(dir.join("recorded"))
        .args(["--", "sh", "-c", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Read on threads that hand their bytes over a channel: a process the
    // step started that outlives it (holding the pipe) is a failure here,
    // not a hang.
    let pipe = |mut r: Box<dyn Read + Send>| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = r.read_to_end(&mut b);
            let _ = tx.send(b);
        });
        rx
    };
    let o = pipe(Box::new(child.stdout.take().unwrap()));
    let e = pipe(Box::new(child.stderr.take().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(120);
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            let err = e.recv_timeout(Duration::from_secs(5)).unwrap_or_default();
            panic!(
                "`quod unit -- sh -c {script:?}` still running after 120 s; stderr:\n{}",
                String::from_utf8_lossy(&err)
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let left = || deadline.saturating_duration_since(Instant::now()) + Duration::from_secs(5);
    let stderr = e.recv_timeout(left()).unwrap_or_else(|_| {
        panic!("`sh -c {script:?}`: quod exited ({status}) but its stderr is still open: a process it started outlives it")
    });
    let stdout = o.recv_timeout(left()).unwrap_or_else(|_| {
        panic!(
            "`sh -c {script:?}`: quod exited ({status}) but its stdout is still open; stderr:\n{}",
            String::from_utf8_lossy(&stderr)
        )
    });
    Output {
        status,
        stdout,
        stderr,
    }
}

#[test]
fn a_unit_records_witnesses_replays_and_reruns_on_a_change() {
    let ok = usable("strace", &["-V"])
        && usable(
            "bwrap",
            &[
                "--dev-bind",
                "/",
                "/",
                "--tmpfs",
                "/tmp",
                "--proc",
                "/proc",
                "--unshare-net",
                "--unshare-pid",
                "--",
                "true",
            ],
        );
    if !ok {
        assert!(
            std::env::var("CI").as_deref() != Ok("true"),
            "no strace or bubblewrap on CI"
        );
        eprintln!("no strace or bubblewrap: skipped");
        return;
    }
    // Not under /tmp: the sandbox's /tmp is its own.
    let base = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let root = fs::canonicalize(base.path()).unwrap().join("repo");
    let dir = fs::canonicalize(base.path()).unwrap().join("units");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("in.txt"), "one\n").unwrap();
    let script = "mkdir -p out && cat in.txt > out/o.txt && echo built";
    let err = |o: &Output| String::from_utf8_lossy(&o.stderr).into_owned();
    let out = |o: &Output| String::from_utf8_lossy(&o.stdout).into_owned();

    // 1. Traced, recorded.
    let o = unit(&dir, &root, script);
    assert!(o.status.success(), "{}", err(&o));
    assert_eq!(out(&o), "built\n");
    if err(&o).contains("cannot reach the checkout as this user can") {
        // Root on the hosted image, where AppArmor denies root's DAC
        // capabilities inside bubblewrap: the unit ran plain, recorded
        // nothing, and its output is the plain run's.
        assert!(!dir.join("recorded").exists());
        assert_eq!(fs::read_to_string(root.join("out/o.txt")).unwrap(), "one\n");
        eprintln!(
            "the sandbox cannot reach the checkout as this user: the unit ran plain, as it must"
        );
        return;
    }
    assert!(err(&o).contains("recorded (first witness"), "{}", err(&o));
    assert!(dir.join("recorded").exists());
    assert_eq!(fs::read_to_string(root.join("out/o.txt")).unwrap(), "one\n");

    // 2. A fresh checkout: the plain second witness agrees.
    fs::remove_dir_all(root.join("out")).unwrap();
    let o = unit(&dir, &root, script);
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("second witness agrees"), "{}", err(&o));

    // 3. Answered: nothing runs, the output is printed and out/ restored.
    fs::remove_dir_all(root.join("out")).unwrap();
    let o = unit(&dir, &root, script);
    assert!(o.status.success(), "{}", err(&o));
    assert!(err(&o).contains("answered from its record"), "{}", err(&o));
    assert_eq!(out(&o), "built\n");
    assert_eq!(fs::read_to_string(root.join("out/o.txt")).unwrap(), "one\n");

    // 4. An input changed: it runs (and records) again.
    fs::remove_dir_all(root.join("out")).unwrap();
    fs::write(root.join("in.txt"), "two\n").unwrap();
    let o = unit(&dir, &root, script);
    assert!(o.status.success(), "{}", err(&o));
    assert!(!err(&o).contains("answered"), "{}", err(&o));
    assert_eq!(fs::read_to_string(root.join("out/o.txt")).unwrap(), "two\n");

    // E2: changed contents with the exact same read names and result
    // are a traced second witness, without an extra plain execution.
    let count = "wc -c < in.txt";
    let o = unit(&dir, &root, count);
    assert!(err(&o).contains("recorded (first witness"), "{}", err(&o));
    fs::write(root.join("in.txt"), "six\n").unwrap();
    let o = unit(&dir, &root, count);
    assert!(err(&o).contains("agrees with another traced record"), "{}", err(&o));
    let o = unit(&dir, &root, count);
    assert!(err(&o).contains("answered from its record"), "{}", err(&o));
    assert_eq!(out(&o).trim(), "4");

    // 5. A unit that reaches for the network always runs.
    let net = "cat in.txt; bash -c 'exec 3<>/dev/tcp/140.82.112.3/443' 2>/dev/null; true";
    let o = unit(&dir, &root, net);
    assert!(o.status.success(), "{}", err(&o));
    let o2 = unit(&dir, &root, net);
    assert!(
        err(&o).contains("always runs") || err(&o2).contains("always runs"),
        "{}\n{}",
        err(&o),
        err(&o2)
    );
}
