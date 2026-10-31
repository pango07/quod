//! Sandbox isolation tests (0a, bubblewrap). Gated on STILLPASS_INTEGRATION=1.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;

use sp_sandbox::{Mount, RunSpec, run};

fn enabled() -> bool {
    let on = std::env::var("STILLPASS_INTEGRATION").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("STILLPASS_INTEGRATION != 1; skipping");
    }
    on
}

fn spec(ws: &Path, out: &Path, script: &str, egress: bool) -> RunSpec {
    RunSpec {
        cmd: ["/bin/sh", "-e", "-c", script].map(String::from).to_vec(),
        cwd: "/workspace".into(),
        env: vec![
            ("PATH".into(), "/usr/local/go/bin:/usr/bin:/bin".into()),
            ("ONLY_ME".into(), "1".into()),
        ],
        mounts: vec![Mount {
            src: ws.into(),
            dst: "/workspace".into(),
            writable: true,
        }],
        egress,
        stdout: out.join("stdout"),
        stderr: out.join("stderr"),
        echo: false,
        identity: None,
    }
}

fn exec(script: &str) -> (i32, String, String) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir(&ws).unwrap();
    let r = run(&spec(&ws, tmp.path(), script, false)).unwrap();
    (
        r.exit_code,
        fs::read_to_string(tmp.path().join("stdout")).unwrap(),
        fs::read_to_string(tmp.path().join("stderr")).unwrap(),
    )
}

#[test]
fn env_is_exactly_the_spec() {
    if !enabled() {
        return;
    }
    let (code, out, _) = exec("env | sort");
    assert_eq!(code, 0);
    // sh itself adds PWD; nothing from the host.
    let names: Vec<&str> = out
        .lines()
        .map(|l| l.split('=').next().unwrap())
        .filter(|n| *n != "PWD" && *n != "SHLVL" && *n != "_")
        .collect();
    assert_eq!(names, ["ONLY_ME", "PATH"], "{out}");
}

#[test]
fn no_network() {
    if !enabled() {
        return;
    }
    let (code, _, err) = exec("curl -sS -m 5 https://example.com >/dev/null");
    assert_ne!(code, 0, "network reachable from sandbox: {err}");
    // No DNS involved: a raw IP must be unreachable too.
    let (code, _, err) = exec("curl -sS -m 5 http://1.1.1.1/ >/dev/null");
    assert_ne!(code, 0, "raw IP reachable from sandbox: {err}");
}

#[test]
fn writes_only_in_workspace_and_tmp() {
    if !enabled() {
        return;
    }
    assert_eq!(exec("echo x > /workspace/ok && echo y > /tmp/ok").0, 0);
    for p in ["/usr/bad", "/bad", "/etc/bad", "/root/bad"] {
        assert_ne!(
            exec(&format!("echo x > {p}")).0,
            0,
            "write to {p} succeeded"
        );
    }
}

#[test]
fn runs_as_unprivileged_runner() {
    if !enabled() {
        return;
    }
    let (code, out, _) = exec("id -u; id -un; touch /workspace/f && stat -c %u /workspace/f");
    assert_eq!(code, 0);
    assert_eq!(out, "1001\nrunner\n1001\n");
}

#[test]
fn host_home_is_not_visible() {
    if !enabled() {
        return;
    }
    let (code, _, _) =
        exec("test -e /root || test -e /home/$(id -un 2>/dev/null)x || test -e /src");
    assert_ne!(code, 0, "host paths visible");
}

#[test]
fn exit_code_and_streams_are_captured() {
    if !enabled() {
        return;
    }
    let (code, out, err) =
        exec("echo out; echo err >&2; head -c 300000 /dev/zero | tr '\\0' a; exit 3");
    assert_eq!(code, 3);
    assert_eq!(out.len(), 4 + 300000);
    assert_eq!(err, "err\n");
}

/// T07 definition of done: with the Schemas env and a module cache filled by
/// step A (egress allowed), step B passes with no network. Runs when
/// STILLPASS_SPIKE_REPO points at a checkout; writes .stillpass/ into it.
#[test]
fn spike_repo_go_test_passes_offline() {
    if !enabled() {
        return;
    }
    let Ok(repo) = std::env::var("STILLPASS_SPIKE_REPO") else {
        eprintln!("STILLPASS_SPIKE_REPO not set; skipping");
        return;
    };
    let env: Vec<(String, String)> = [
        ("CGO_ENABLED", "0"),
        ("GOCACHE", "/workspace/.stillpass/gocache"),
        ("GOFLAGS", "-mod=mod"),
        ("GOMODCACHE", "/workspace/.stillpass/gomod"),
        ("GOTOOLCHAIN", "local"),
        ("HOME", "/workspace/.stillpass/home"),
        ("LANG", "C.UTF-8"),
        ("PATH", "/usr/local/go/bin:/usr/bin:/bin"),
        ("TZ", "UTC"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let tmp = tempfile::tempdir().unwrap();
    let mut a = spec(Path::new(&repo), tmp.path(), "go mod download", true);
    a.env = env.clone();
    a.env
        .push(("GOPROXY".into(), "https://proxy.golang.org,direct".into()));
    a.echo = true;
    let ra = run(&a).unwrap();
    assert_eq!(ra.exit_code, 0, "step A failed");
    // Step B gets a fresh copy of the cache-less state: GOCACHE emptied.
    let _ = fs::remove_dir_all(Path::new(&repo).join(".stillpass/gocache"));
    let mut b = spec(Path::new(&repo), tmp.path(), "go test ./...", false);
    b.env = env;
    b.env.push(("GOPROXY".into(), "off".into()));
    let rb = run(&b).unwrap();
    let out = fs::read_to_string(tmp.path().join("stdout")).unwrap();
    let err = fs::read_to_string(tmp.path().join("stderr")).unwrap();
    eprintln!(
        "step A {:?}, step B {:?}, {} ok lines\n{}",
        ra.wall,
        rb.wall,
        out.lines().filter(|l| l.starts_with("ok")).count(),
        err.lines().take(20).collect::<Vec<_>>().join("\n")
    );
    assert_eq!(
        rb.exit_code,
        0,
        "go test ./... failed offline:\n{}",
        out.lines()
            .filter(|l| l.starts_with("FAIL") || l.starts_with("---"))
            .take(40)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn a_sandbox_that_never_starts_the_step_is_an_error_not_a_result() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let mut s = spec(
        &tmp.path().join("does-not-exist"),
        tmp.path(),
        "exit 0",
        false,
    );
    s.cwd = "/workspace".into();
    match run(&s) {
        Err(sp_sandbox::SandboxError::DidNotStart { .. }) => {}
        other => panic!("expected DidNotStart, got {other:?}"),
    }
}

#[test]
fn signal_deaths_are_reported() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir(&ws).unwrap();
    let r = run(&spec(&ws, tmp.path(), "kill -9 $$", false)).unwrap();
    assert_eq!((r.signal, r.exit_code), (Some(9), 137));
}
