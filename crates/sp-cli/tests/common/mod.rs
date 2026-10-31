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
        Env { tmp, repo }
    }

    pub fn state(&self) -> PathBuf {
        self.tmp.path().join("state")
    }

    pub fn stillpass(&self, args: &[&str]) -> Out {
        let out = Command::new(env!("CARGO_BIN_EXE_stillpass"))
            .arg("--state")
            .arg(self.state())
            .args(args)
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
