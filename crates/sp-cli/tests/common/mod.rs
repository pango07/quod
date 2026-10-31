//! Helpers for CLI integration tests: tiny Go, Node and Python modules and a
//! runner for the stillpass binary. Gated on STILLPASS_INTEGRATION=1 (Linux,
//! FUSE, bwrap, the toolchain).
#![allow(dead_code)] // each test binary uses a subset

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sp_key::Toolchain;

pub fn enabled() -> bool {
    let on =
        cfg!(target_os = "linux") && std::env::var("STILLPASS_INTEGRATION").is_ok_and(|v| v == "1");
    if !on {
        eprintln!("needs Linux and STILLPASS_INTEGRATION=1; skipping");
    }
    on
}

/// Is the toolchain's binary where the profile's PATH expects it?
pub fn toolchain_installed(tc: Toolchain) -> bool {
    let bin = match tc {
        Toolchain::Go => "/usr/local/go/bin/go",
        Toolchain::Node => "/usr/local/node/bin/node",
        Toolchain::Python => "/usr/local/python/bin/python3",
    };
    let ok = Path::new(bin).is_file();
    if !ok {
        eprintln!("{bin} is not installed; skipping the {tc} fixture");
    }
    ok
}

fn write(dir: &Path, p: &str, s: &str) {
    let p = dir.join(p);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, s).unwrap();
}

fn init_git(dir: &Path) {
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

/// A Go module whose tests read a fixture, list a directory and probe for an
/// optional config file.
pub fn go_module(dir: &Path) {
    let w = |p: &str, s: &str| write(dir, p, s);
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
    init_git(dir);
}

/// A Node package with an installed dependency tree (a hand-made
/// `node_modules`, so no npm is needed offline) whose test reads a fixture,
/// lists the source directory and probes for an optional config file. The
/// test step is `npm test`, so npm itself runs under the profile.
pub fn node_package(dir: &Path) {
    let w = |p: &str, s: &str| write(dir, p, s);
    w(
        "package.json",
        r#"{
  "name": "tiny",
  "version": "0.0.0",
  "private": true,
  "scripts": { "test": "node test/run.js" },
  "dependencies": { "dep": "1.0.0" }
}
"#,
    );
    w("README.md", "# tiny\n");
    w(
        "node_modules/dep/package.json",
        "{ \"name\": \"dep\", \"version\": \"1.0.0\", \"main\": \"index.js\" }\n",
    );
    w(
        "node_modules/dep/index.js",
        "module.exports = { name: 'dep' };\n",
    );
    w(
        "node_modules/.package-lock.json",
        "{ \"name\": \"tiny\", \"lockfileVersion\": 3, \"packages\": {} }\n",
    );
    w("src/add.js", "module.exports = { add: (a, b) => a + b };\n");
    w(
        "test/run.js",
        r#"const assert = require('assert');
const fs = require('fs');
const path = require('path');
const dep = require('dep');
const { add } = require('../src/add');
assert.strictEqual(add(2, 3), 5);
assert.strictEqual(dep.name, 'dep');
const fixtures = path.join(__dirname, 'fixtures');
assert.strictEqual(fs.readFileSync(path.join(fixtures, 'fixture.txt'), 'utf8').trim(), 'hello');
const cases = fs.readdirSync(path.join(fixtures, 'cases'));
assert.ok(cases.length >= 2);
const sources = fs.readdirSync(path.join(__dirname, '..', 'src'));
if (fs.existsSync(path.join(fixtures, 'override.json'))) console.log('override present');
console.log('ok', cases.length, 'cases', sources.length, 'sources');
"#,
    );
    w("test/fixtures/fixture.txt", "hello\n");
    w("test/fixtures/cases/a.txt", "a\n");
    w("test/fixtures/cases/b.txt", "b\n");
    init_git(dir);
}

/// A Python package with a venv at the profile's path (made offline, without
/// pip) whose unittest suite reads a fixture, lists a directory and probes
/// for an optional config file.
pub fn python_package(dir: &Path) {
    let w = |p: &str, s: &str| write(dir, p, s);
    w(
        "pyproject.toml",
        "[project]\nname = \"tiny\"\nversion = \"0.0.0\"\n",
    );
    w("README.md", "# tiny\n");
    w("tiny/__init__.py", "");
    w("tiny/calc.py", "def add(a, b):\n    return a + b\n");
    w("tests/__init__.py", "");
    w(
        "tests/test_calc.py",
        r#"import os
import unittest

from tiny.calc import add

HERE = os.path.dirname(__file__)


class CalcTest(unittest.TestCase):
    def test_add(self):
        self.assertEqual(add(2, 3), 5)

    def test_fixture(self):
        with open(os.path.join(HERE, "fixtures", "fixture.txt")) as f:
            self.assertEqual(f.read().strip(), "hello")

    def test_listing(self):
        cases = os.listdir(os.path.join(HERE, "fixtures", "cases"))
        self.assertGreaterEqual(len(cases), 2)
        print("ok", len(cases), "cases")

    def test_optional_config(self):
        if os.path.exists(os.path.join(HERE, "fixtures", "override.json")):
            print("override present")
"#,
    );
    w("tests/fixtures/fixture.txt", "hello\n");
    w("tests/fixtures/cases/a.txt", "a\n");
    w("tests/fixtures/cases/b.txt", "b\n");
    let st = Command::new("/usr/local/python/bin/python3")
        .args(["-m", "venv", "--without-pip"])
        .arg(dir.join(".stillpass/venv"))
        .status()
        .unwrap();
    assert!(st.success(), "python3 -m venv");
    write(dir, ".git/info/exclude", ".stillpass/\n");
    init_git(dir);
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
    /// Passed as --toolchain to every run; detection is used when `None`.
    pub toolchain: Option<Toolchain>,
    /// Flags `run_step` adds to every `stillpass run`. The default puts the
    /// matcher's acceptance criteria (A1-A7, W) back on the spike's terms:
    /// replay mode, a result replays after one execution, no audit
    /// sampling. The trust suite clears this and sets the policy per call.
    pub run_defaults: Vec<String>,
}

/// The spike's pre-trust semantics: a matching result replays at once.
pub fn legacy_policy() -> Vec<String> {
    ["--mode", "replay", "--proofs", "1", "--audit", "0"]
        .map(String::from)
        .to_vec()
}

impl Env {
    /// The tiny Go module.
    pub fn new() -> Env {
        Env::fixture(Toolchain::Go)
    }

    /// The tiny fixture for a toolchain, run under its profile explicitly.
    pub fn fixture(tc: Toolchain) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        match tc {
            Toolchain::Go => go_module(&repo),
            Toolchain::Node => node_package(&repo),
            Toolchain::Python => python_package(&repo),
        }
        let env = Env {
            tmp,
            repo,
            toolchain: Some(tc),
            run_defaults: legacy_policy(),
        };
        env.seed_rootfs();
        env
    }

    /// A scratch copy of an existing checkout (with .git and .stillpass).
    pub fn from_copy(src: &Path, toolchain: Option<Toolchain>) -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let st = Command::new("cp")
            .arg("-a")
            .arg(src)
            .arg(&repo)
            .status()
            .unwrap();
        assert!(st.success(), "copy {}", src.display());
        let env = Env {
            tmp,
            repo,
            toolchain,
            run_defaults: legacy_policy(),
        };
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

    /// Commits everything. `--no-verify`: the repo's own hooks are its
    /// developers' tooling, not CI's (luxon's husky pre-commit runs
    /// lint-staged, whose prettier rewrites the staged file and creates
    /// node_modules/.cache, changing the tree the test meant to change by
    /// one file).
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
                "--no-verify",
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
        if let Some(tc) = self.toolchain {
            args.extend(["--toolchain", tc.name()]);
        }
        args.extend(self.run_defaults.iter().map(String::as_str));
        args.extend_from_slice(extra);
        self.stillpass(&args)
    }

    /// `stillpass status --repo <repo>` as text.
    pub fn status(&self) -> String {
        let o = self.stillpass(&["status", "--repo", self.repo.to_str().unwrap()]);
        assert_eq!(o.code, 0, "{}", o.stderr);
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    /// The number of manifests in the ledger.
    pub fn manifests(&self) -> u64 {
        let out = Command::new("sqlite3")
            .arg(self.state().join("ledger.sqlite"))
            .arg("SELECT count(*) FROM manifests")
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
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
