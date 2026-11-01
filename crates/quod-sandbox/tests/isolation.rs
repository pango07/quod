//! Sandbox isolation tests (0a, bubblewrap). Gated on QUOD_INTEGRATION=1.
#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};

use quod_sandbox::{Mount, RootFs, RunSpec, run};

fn enabled() -> bool {
    let on = std::env::var("QUOD_INTEGRATION").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("QUOD_INTEGRATION != 1; skipping");
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
        rootfs: None,
        identity: None,
        uid: None,
        gid: None,
        net_probe: false,
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
fn env_names_that_are_not_identifiers_reach_the_step() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir(&ws).unwrap();
    let mut s = spec(&ws, tmp.path(), "", false);
    s.cmd = vec!["/usr/bin/env".into()];
    s.env
        .push(("INPUT_GO-VERSION-FILE".into(), "go.mod".into()));
    let r = run(&s).unwrap();
    assert_eq!(r.exit_code, 0);
    let out = fs::read_to_string(tmp.path().join("stdout")).unwrap();
    assert!(
        out.lines().any(|l| l == "INPUT_GO-VERSION-FILE=go.mod"),
        "{out}"
    );
    assert!(out.lines().any(|l| l == "ONLY_ME=1"), "{out}");
}

#[test]
fn network_attempts_are_counted() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir(&ws).unwrap();
    let rc = tmp.path().join("resolv.conf");
    fs::write(&rc, "nameserver 192.0.2.1\n").unwrap();
    let probe = |script: &str| {
        let mut s = spec(&ws, tmp.path(), script, false);
        s.net_probe = true;
        s.mounts.push(Mount {
            src: rc.clone(),
            dst: "/etc/resolv.conf".into(),
            writable: false,
        });
        run(&s).unwrap()
    };
    assert_eq!(probe("true").net_attempts, Some(0));
    let r = probe("curl -sS -m 5 http://1.1.1.1/ || true");
    assert_eq!(r.exit_code, 0, "the step itself succeeds");
    assert!(r.net_attempts.unwrap() > 0, "{r:?}");
    assert!(
        probe("getent hosts example.com || true")
            .net_attempts
            .unwrap()
            > 0
    );
    // Loopback is not the network.
    assert_eq!(
        probe("curl -sS -m 5 http://127.0.0.1:9/ || true").net_attempts,
        Some(0)
    );
    // An exit status survives the wrapper.
    assert_eq!(probe("exit 7").exit_code, 7);
}

#[test]
fn uid_gid_and_hostname() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir(&ws).unwrap();
    let mut s = spec(
        &ws,
        tmp.path(),
        "id -u; id -g; hostname; grep runner /etc/passwd",
        false,
    );
    s.uid = Some(1234);
    s.gid = Some(4321);
    let r = run(&s).unwrap();
    assert_eq!(r.exit_code, 0);
    let out = fs::read_to_string(tmp.path().join("stdout")).unwrap();
    assert_eq!(
        out,
        format!(
            "1234\n4321\n{}\nrunner:x:1234:4321:runner:/workspace/.quod/home:/bin/sh\n",
            quod_sandbox::HOSTNAME
        )
    );
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
/// QUOD_SPIKE_REPO points at a checkout; writes .quod/ into it.
#[test]
fn spike_repo_go_test_passes_offline() {
    if !enabled() {
        return;
    }
    let Ok(repo) = std::env::var("QUOD_SPIKE_REPO") else {
        eprintln!("QUOD_SPIKE_REPO not set; skipping");
        return;
    };
    let env: Vec<(String, String)> = [
        ("CGO_ENABLED", "0"),
        ("GOCACHE", "/workspace/.quod/gocache"),
        ("GOFLAGS", "-mod=mod"),
        ("GOMODCACHE", "/workspace/.quod/gomod"),
        ("GOTOOLCHAIN", "local"),
        ("HOME", "/workspace/.quod/home"),
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
    let _ = fs::remove_dir_all(Path::new(&repo).join(".quod/gocache"));
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
        Err(quod_sandbox::SandboxError::DidNotStart { .. }) => {}
        other => panic!("expected DidNotStart, got {other:?}"),
    }
}

/// A minimal rootfs made from the host's own `sh`, `cat` and `id` plus the
/// libraries `ldd` names, laid out at their host paths (with the host's
/// top-level symlinks such as `bin -> usr/bin`), and an /etc of our making.
fn mini_rootfs(dir: &Path) {
    // Where a host path lives in the rootfs: through the host's top-level
    // symlink if there is one (`/lib/x` is `usr/lib/x` on a merged-usr host).
    let place = |host: &Path| -> PathBuf {
        let rel = host.strip_prefix("/").unwrap();
        let mut parts = rel.components();
        let first = parts.next().unwrap().as_os_str().to_owned();
        let base = match fs::read_link(Path::new("/").join(&first)) {
            Ok(target) => dir.join(target),
            Err(_) => dir.join(&first),
        };
        base.join(parts.as_path())
    };
    // Copies a host file (the content of its final target) and, when the
    // host path itself is a symlink, that symlink too.
    let bring = |host: &Path| {
        let real = fs::canonicalize(host).unwrap();
        let dest = place(&real);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        if !dest.exists() {
            fs::copy(&real, &dest).unwrap_or_else(|e| panic!("copy {}: {e}", real.display()));
        }
        if let Ok(target) = fs::read_link(host) {
            let link = place(host);
            fs::create_dir_all(link.parent().unwrap()).unwrap();
            if fs::symlink_metadata(&link).is_err() {
                std::os::unix::fs::symlink(target, link).unwrap();
            }
        }
    };
    for top in ["bin", "sbin", "lib", "lib64", "lib32"] {
        if let Ok(target) = fs::read_link(Path::new("/").join(top)) {
            std::os::unix::fs::symlink(target, dir.join(top)).unwrap();
        }
    }
    for bin in ["sh", "cat", "id"] {
        let out = std::process::Command::new("sh")
            .args(["-c", &format!("command -v {bin}")])
            .output()
            .unwrap();
        let path = PathBuf::from(String::from_utf8(out.stdout).unwrap().trim());
        bring(&path);
        let ldd = std::process::Command::new("ldd")
            .arg(&path)
            .output()
            .unwrap();
        for line in String::from_utf8_lossy(&ldd.stdout).lines() {
            // "libc.so.6 => /lib/aarch64-linux-gnu/libc.so.6 (0x...)" or
            // "/lib/ld-linux-aarch64.so.1 (0x...)".
            let lib = line.split_once("=>").map_or(line, |(_, rhs)| rhs).trim();
            if let Some(lib) = lib.split_whitespace().next()
                && lib.starts_with('/')
            {
                bring(Path::new(lib));
            }
        }
    }
    fs::create_dir_all(dir.join("etc")).unwrap();
    fs::write(dir.join("etc/os-release"), "ID=quod-test\n").unwrap();
    fs::write(dir.join("etc/passwd"), "x").unwrap();
    fs::write(dir.join("etc/group"), "x").unwrap();
    fs::write(dir.join("etc/resolv.conf"), "").unwrap();
    fs::write(dir.join("etc/only-in-rootfs"), "yes\n").unwrap();
    for d in ["dev", "proc", "sys", "tmp", "home", "opt", "workspace"] {
        fs::create_dir_all(dir.join(d)).unwrap();
    }
    fs::write(dir.join("tmp/leak"), "x").unwrap();
    fs::write(dir.join("dev/leak"), "x").unwrap();
}

const ROOTFS_PROBE: &str = "cat /etc/os-release; cat /etc/only-in-rootfs; cat /proc/sys/kernel/hostname; \
    id -u; id -un; \
    test -e /usr/local/go && echo LEAK-go; test -e /etc/fuse.conf && echo LEAK-fuse; \
    test -e /usr/bin/bwrap && echo LEAK-bwrap; test -e /tmp/leak && echo LEAK-tmp; \
    test -e /dev/leak && echo LEAK-dev; test -e /src && echo LEAK-src; test -e /sys/kernel && echo LEAK-sys; \
    test -d /proc/self && test -c /dev/null && test -d /tmp && test -d /workspace && echo mounts-ok; \
    echo x > /workspace/ok && echo ws-writable; echo y > /opt/no && echo root-writable; true";

/// With a rootfs, the step sees that filesystem and nothing of the host's:
/// not /etc, not /usr/local, not /sys, not the hostname. Read-only, writes
/// outside the workspace fail; read-write (an overlay's merged dir), they
/// land in the directory for the diff.
#[test]
fn rootfs_replaces_the_host_userland() {
    if !enabled() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("rootfs");
    fs::create_dir(&root).unwrap();
    mini_rootfs(&root);
    let ws = tmp.path().join("ws");
    fs::create_dir(&ws).unwrap();
    let mut s = spec(&ws, tmp.path(), ROOTFS_PROBE, false);
    s.rootfs = Some(RootFs {
        dir: root.clone(),
        writable: false,
    });
    let r = run(&s).unwrap();
    let out = fs::read_to_string(tmp.path().join("stdout")).unwrap();
    let err = fs::read_to_string(tmp.path().join("stderr")).unwrap();
    assert_eq!(r.exit_code, 0, "{out}\n{err}");
    assert_eq!(
        out, "ID=quod-test\nyes\nquod\n1001\nrunner\nmounts-ok\nws-writable\n",
        "{err}"
    );
    assert!(!root.join("opt/no").exists());

    // Read-write: writes outside the workspace succeed and land in the dir,
    // /tmp included (it is the rootfs's own, not a tmpfs: what a step
    // leaves there is an output).
    s.rootfs = Some(RootFs {
        dir: root.clone(),
        writable: true,
    });
    s.cmd = vec![
        "/bin/sh".into(),
        "-c".into(),
        format!("{ROOTFS_PROBE}; echo t > /tmp/new; cat /tmp/leak"),
    ];
    let r = run(&s).unwrap();
    let out = fs::read_to_string(tmp.path().join("stdout")).unwrap();
    assert_eq!(r.exit_code, 0);
    assert!(
        out.contains("LEAK-tmp\n") && out.ends_with("mounts-ok\nws-writable\nroot-writable\nx"),
        "/tmp is the rootfs's: {out}"
    );
    assert_eq!(fs::read_to_string(root.join("opt/no")).unwrap(), "y\n");
    assert_eq!(fs::read_to_string(root.join("tmp/new")).unwrap(), "t\n");
    assert!(
        !root.join("dev/.quod-started").exists(),
        "the sentinel leaves no trace"
    );

    // Host mode, same probe: the host's /etc shows through (the control).
    let s = spec(&ws, tmp.path(), "cat /etc/os-release | head -1", false);
    run(&s).unwrap();
    let host = fs::read_to_string(tmp.path().join("stdout")).unwrap();
    assert!(!host.contains("quod-test"), "{host}");
}

#[test]
fn hostname_is_fixed() {
    if !enabled() {
        return;
    }
    let (code, out, _) = exec("cat /proc/sys/kernel/hostname");
    assert_eq!((code, out.as_str()), (0, "quod\n"));
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

/// A limit kills the step and says so; its children die with bubblewrap.
#[test]
fn limits_kill_the_step_and_report_why() {
    if !enabled() {
        return;
    }
    use quod_sandbox::{Killed, Limits, run_with};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    fs::create_dir(&ws).unwrap();
    let t = Instant::now();
    let r = run_with(
        &spec(&ws, tmp.path(), "sleep 30 & sleep 30", false),
        &Limits {
            timeout: Some(Duration::from_millis(500)),
            abort: None,
        },
    )
    .unwrap();
    assert_eq!(r.killed, Some(Killed::Timeout));
    assert!(r.signal.is_some(), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());

    let abort = Arc::new(AtomicBool::new(false));
    let flag = abort.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        flag.store(true, Ordering::SeqCst);
    });
    let t = Instant::now();
    let r = run_with(
        &spec(&ws, tmp.path(), "sleep 30", false),
        &Limits {
            timeout: Some(Duration::from_secs(60)),
            abort: Some(abort),
        },
    )
    .unwrap();
    assert_eq!(r.killed, Some(Killed::Aborted));
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());

    // A step that finishes in time is untouched.
    let r = run_with(
        &spec(&ws, tmp.path(), "exit 0", false),
        &Limits {
            timeout: Some(Duration::from_secs(60)),
            abort: None,
        },
    )
    .unwrap();
    assert_eq!((r.killed, r.exit_code), (None, 0));
}
