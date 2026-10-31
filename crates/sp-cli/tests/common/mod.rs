//! Helpers for CLI integration tests: a tiny Go module and a runner for the
//! stillpass binary. Gated on STILLPASS_INTEGRATION=1 (Linux, FUSE, bwrap, Go).
#![allow(dead_code)] // each test binary uses a subset

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn enabled() -> bool {
    let on =
        cfg!(target_os = "linux") && std::env::var("STILLPASS_INTEGRATION").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("needs Linux and STILLPASS_INTEGRATION=1; skipping");
    }
    on
}

/// A Go module whose tests read a fixture, list a directory and probe for an
/// optional config file.
pub fn go_module(dir: &Path) {
    let w = |p: &str, s: &str| {
        let p = dir.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    };
    w("go.mod", "module example.com/tiny\n\ngo 1.27\n");
    w("README.md", "# tiny\n");
    w(
        "calc/calc.go",
        "package calc\n\nfunc Add(a, b int) int { return a + b }\n",
    );
    w(
        "calc/calc_test.go",
        r#"package calc

import (
	"os"
	"strings"
	"testing"
)

func TestAdd(t *testing.T) {
	if Add(2, 3) != 5 {
		t.Fatal("math")
	}
}

func TestFixture(t *testing.T) {
	b, err := os.ReadFile("testdata/fixture.txt")
	if err != nil || strings.TrimSpace(string(b)) != "hello" {
		t.Fatalf("fixture: %q %v", b, err)
	}
}

func TestListing(t *testing.T) {
	es, err := os.ReadDir("testdata/cases")
	if err != nil || len(es) < 2 {
		t.Fatalf("cases: %v %v", es, err)
	}
	t.Logf("%d cases", len(es))
}

func TestOptionalConfig(t *testing.T) {
	if _, err := os.Stat("testdata/override.json"); err == nil {
		t.Log("override present")
	}
}
"#,
    );
    w("calc/testdata/fixture.txt", "hello\n");
    w("calc/testdata/cases/a.txt", "a\n");
    w("calc/testdata/cases/b.txt", "b\n");
    w(
        "other/other.go",
        "package other\n\nconst Name = \"other\"\n",
    );
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["add", "-A"]);
    git(
        dir,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "init",
        ],
    );
}

pub fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(st.success(), "git {args:?}");
}

pub struct Out {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Out {
    pub fn hit(&self) -> bool {
        self.stderr.contains("STILLPASS  REPLAYED")
    }

    pub fn executed(&self) -> bool {
        self.stderr.contains("STILLPASS  EXECUTED")
    }
}

pub struct Env {
    pub tmp: tempfile::TempDir,
    pub repo: PathBuf,
}

impl Env {
    pub fn new() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        go_module(&repo);
        let env = Env { tmp, repo };
        env.seed_rootfs();
        env
    }

    /// A scratch copy of an existing checkout (with .git and .stillpass).
    pub fn from_copy(src: &Path) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let st = Command::new("cp")
            .arg("-a")
            .arg(src)
            .arg(&repo)
            .status()
            .unwrap();
        assert!(st.success(), "copy {}", src.display());
        let env = Env { tmp, repo };
        env.seed_rootfs();
        env
    }

    /// Rootfs mode: every `stillpass run` child inherits STILLPASS_ROOTFS,
    /// so that rootfs must be in this environment's own store. It is
    /// imported from STILLPASS_ROOTFS_SRC (the export tar or directory) and
    /// must come out at the digest STILLPASS_ROOTFS names.
    fn seed_rootfs(&self) {
        seed_rootfs_in(&self.state());
    }

    pub fn commit(&self, msg: &str) {
        git(&self.repo, &["add", "-A"]);
        git(
            &self.repo,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "-m",
                msg,
            ],
        );
    }

    pub fn state(&self) -> PathBuf {
        self.tmp.path().join("state")
    }

    pub fn stillpass(&self, args: &[&str]) -> Out {
        self.stillpass_in(&self.state(), args, &[])
    }

    /// `stillpass` with its own state dir and extra environment.
    pub fn stillpass_in(&self, state: &Path, args: &[&str], envs: &[(&str, &str)]) -> Out {
        // Every state dir a test runs in has the rootfs, not only the
        // default one (a test comparing transports uses one state per
        // transport).
        if args.first() != Some(&"rootfs") {
            seed_rootfs_in(state);
        }
        let out = Command::new(env!("CARGO_BIN_EXE_stillpass"))
            .arg("--state")
            .arg(state)
            .args(args)
            .envs(envs.iter().copied())
            .output()
            .unwrap();
        let o = Out {
            code: out.status.code().unwrap_or(-1),
            stdout: out.stdout,
            stderr: String::from_utf8_lossy(&out.stderr).into(),
        };
        eprintln!(
            "$ stillpass {}\n{}",
            args.join(" "),
            o.stderr
                .lines()
                .filter(|l| l.starts_with("STILLPASS") || l.starts_with("  "))
                .collect::<Vec<_>>()
                .join("\n")
        );
        o
    }

    pub fn run_step(&self, step: &str, extra: &[&str]) -> Out {
        let repo = self.repo.to_str().unwrap();
        let mut args = vec!["run", "--repo", repo, "--step", step];
        args.extend_from_slice(extra);
        self.stillpass(&args)
    }

    pub fn write(&self, rel: &str, content: &str) {
        let p = self.repo.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }
}

/// Rootfs mode: every `stillpass run` child inherits STILLPASS_ROOTFS, so
/// that rootfs must be in the state dir's own store. It is imported from
/// STILLPASS_ROOTFS_SRC (the export tar or directory), once per state dir,
/// and must come out at the digest STILLPASS_ROOTFS names.
pub fn seed_rootfs_in(state: &Path) {
    let Ok(digest) = std::env::var("STILLPASS_ROOTFS") else {
        return;
    };
    let hex = digest.strip_prefix("b3:").unwrap_or(&digest);
    if state.join(format!("cas/rootfs/index/{hex}.json")).is_file() {
        return;
    }
    let src = std::env::var("STILLPASS_ROOTFS_SRC").expect(
        "STILLPASS_ROOTFS is set: STILLPASS_ROOTFS_SRC must name the export it was imported from",
    );
    let out = Command::new(env!("CARGO_BIN_EXE_stillpass"))
        .arg("--state")
        .arg(state)
        .args(["rootfs", "import", &src])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "rootfs import: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        digest,
        "STILLPASS_ROOTFS_SRC imports to another digest"
    );
}

/// The structured events a state dir's runs wrote, oldest first.
pub fn events(state: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(state.join("events.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// The last event with this message.
pub fn last_event(state: &Path, message: &str) -> Option<serde_json::Value> {
    events(state)
        .into_iter()
        .rev()
        .find(|e| e["message"] == message)
}

/// The raw JSON line of the last event with this message, as the binary
/// wrote it.
pub fn last_event_line(state: &Path, message: &str) -> Option<String> {
    let needle = format!("\"message\":\"{message}\"");
    fs::read_to_string(state.join("events.jsonl"))
        .ok()?
        .lines()
        .rev()
        .find(|l| l.contains(&needle))
        .map(str::to_string)
}

/// A minimal root filesystem made from this host's `bins` (plus `sh`) and
/// the libraries `ldd` names, at their host paths (with the host's top-level
/// symlinks such as `bin -> usr/bin`), an /etc of our making and the usual
/// top-level directories: enough to import with `stillpass rootfs import`
/// and run shell steps in, without the real rootfs.
pub fn mini_rootfs(dir: &Path, bins: &[&str]) {
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
    fs::create_dir_all(dir).unwrap();
    for top in ["bin", "sbin", "lib", "lib64", "lib32"] {
        if let Ok(target) = fs::read_link(Path::new("/").join(top)) {
            std::os::unix::fs::symlink(target, dir.join(top)).unwrap();
        }
    }
    for bin in std::iter::once(&"sh").chain(bins) {
        let out = Command::new("sh")
            .args(["-c", &format!("command -v {bin}")])
            .output()
            .unwrap();
        let path = PathBuf::from(String::from_utf8(out.stdout).unwrap().trim());
        assert!(path.is_absolute(), "{bin} not found on this host");
        bring(&path);
        let ldd = Command::new("ldd").arg(&path).output().unwrap();
        for line in String::from_utf8_lossy(&ldd.stdout).lines() {
            let lib = line.split_once("=>").map_or(line, |(_, rhs)| rhs).trim();
            if let Some(lib) = lib.split_whitespace().next()
                && lib.starts_with('/')
            {
                bring(Path::new(lib));
            }
        }
    }
    let w = |p: &str, s: &str| {
        let p = dir.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    };
    w("etc/os-release", "ID=stillpass-mini\n");
    w("etc/passwd", "root:x:0:0:root:/root:/bin/sh\n");
    w("etc/group", "root:x:0:\n");
    w("opt/replace-me", "an image file\n");
    for d in ["dev", "proc", "sys", "tmp", "opt", "var", "workspace"] {
        fs::create_dir_all(dir.join(d)).unwrap();
    }
}
