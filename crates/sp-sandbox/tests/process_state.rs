//! The step's process state is fixed, not inherited: a parent's umask,
//! ignored signals, blocked signals, open descriptors and soft rlimits do not
//! reach it. Gated on STILLPASS_INTEGRATION=1. Its own test binary: the test
//! re-executes itself under a hostile shell, and nothing else may run in a
//! process whose state it changes.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::Command;

use sp_sandbox::{Mount, RunSpec, effective_rlimits, run};

const CHILD: &str = "STILLPASS_PROCESS_STATE_CHILD";

/// What the step reports about its own process state, with sh builtins and
/// files it creates (their modes are what the umask leaves).
const PROBE: &str = "umask; \
    while read -r k v rest; do case $k in SigIgn:|SigBlk:|SigCgt:) echo \"$k $v\";; esac; done < /proc/self/status; \
    for fd in /proc/self/fd/*; do echo \"fd ${fd##*/}\"; done; \
    echo \"nofile $(ulimit -n) $(ulimit -Hn)\"; echo \"stack $(ulimit -s)\"; echo \"core $(ulimit -c)\"; \
    echo x > /workspace/f; mkdir /workspace/d; \
    ls -ld /workspace/f /workspace/d | while read -r mode rest; do echo \"$mode\"; done";

/// Not a test of its own: the body the parent test runs in a child process
/// (it returns at once unless that child asked for it).
#[test]
fn process_state_child() {
    let Ok(out) = std::env::var(CHILD) else {
        return;
    };
    let out = Path::new(&out);
    let ws = out.join("ws");
    fs::create_dir_all(&ws).unwrap();
    let spec = RunSpec {
        cmd: ["/bin/sh", "-c", PROBE].map(String::from).to_vec(),
        cwd: "/workspace".into(),
        env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        mounts: vec![Mount {
            src: ws.clone(),
            dst: "/workspace".into(),
            writable: true,
        }],
        egress: false,
        stdout: out.join("stdout"),
        stderr: out.join("stderr"),
        echo: false,
        rootfs: None,
        identity: None,
    };
    let r = run(&spec).unwrap();
    assert_eq!(
        r.exit_code,
        0,
        "{}",
        fs::read_to_string(out.join("stderr")).unwrap()
    );
}

fn probe(dir: &Path, hostile: bool) -> String {
    fs::create_dir_all(dir).unwrap();
    let exe = std::env::current_exe().unwrap();
    // The hostile parent: umask 077, SIGINT and SIGHUP ignored, an
    // inheritable descriptor 7, lower soft limits for files and stack.
    let setup = if hostile {
        "umask 077; trap '' INT HUP; exec 7</dev/null; ulimit -S -n 512; ulimit -S -s 16384; "
    } else {
        ""
    };
    let st = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "{setup}exec \"$0\" --exact process_state_child --nocapture"
        ))
        .arg(&exe)
        .env(CHILD, dir)
        .status()
        .unwrap();
    assert!(
        st.success(),
        "child under {} parent failed",
        if hostile { "a hostile" } else { "a plain" }
    );
    fs::read_to_string(dir.join("stdout")).unwrap()
}

#[test]
fn a_parents_process_state_does_not_reach_the_step() {
    let on = std::env::var("STILLPASS_INTEGRATION").is_ok_and(|v| v == "1");
    if !on || std::env::var(CHILD).is_ok() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let plain = probe(&tmp.path().join("plain"), false);
    let hostile = probe(&tmp.path().join("hostile"), true);
    assert_eq!(hostile, plain, "the parent's state leaked into the step");
    let hard_nofile = effective_rlimits()
        .into_iter()
        .find(|(n, ..)| *n == "NOFILE")
        .map(|(_, _, h)| h)
        .unwrap();
    for want in [
        "0022\n".to_string(),
        "SigIgn: 0000000000000000\n".into(),
        "SigBlk: 0000000000000000\n".into(),
        format!("nofile 1024 {hard_nofile}\n"),
        "stack 8192\n".into(),
        "core 0\n".into(),
        "drwxr-xr-x\n-rw-r--r--\n".into(),
    ] {
        assert!(plain.contains(&want), "{want:?} not in:\n{plain}");
    }
    // Descriptors: 0, 1, 2 and the one the shell's glob opened, nothing else.
    let fds: Vec<&str> = plain.lines().filter(|l| l.starts_with("fd ")).collect();
    assert!(fds.len() <= 4 && !fds.contains(&"fd 7"), "{fds:?}");
}
