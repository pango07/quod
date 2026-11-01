//! The served git index (DECISIONS.md, exp/git-index). Through `quod run`:
//! - `git status --porcelain` and the `vcs.*` lines `go version -m` prints
//!   for a `go build` are the same in the served workspace as in the plain
//!   checkout, clean and dirty (modified, staged, deleted, untracked);
//! - over a clean served checkout git takes every entry's stat data and
//!   reads no file (`refresh/sum_scan:0` in its trace2), as in a fresh
//!   checkout;
//! - what replays: a README commit replays a step that only finds the
//!   repository (on a branch, HEAD names the branch, not the commit), and
//!   misses for `go build`, whose binary embeds the commit; editing a .go
//!   file or go.mod misses for both.
//!
//! On the tiny module below by default, on the spike repo too when
//! QUOD_SPIKE_REPO is set (its go build of ./cmd/gh). Gated on
//! QUOD_INTEGRATION=1.

mod common;
use std::path::Path;
use std::process::Command;

use common::*;
use quod_key::Toolchain;

/// A module with a main package, a test that finds the repository the way
/// cli/cli's tests do (a git command in the checkout), and a README.
fn fixture(dir: &Path) {
    let w = |p: &str, s: &str| {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    };
    w("go.mod", "module example.com/vcs\n\ngo 1.27\n");
    w("README.md", "# vcs\n");
    w(
        "main.go",
        "package main\n\nimport \"example.com/vcs/calc\"\n\nfunc main() { println(calc.Add(1, 2)) }\n",
    );
    w(
        "calc/calc.go",
        "package calc\n\nfunc Add(a, b int) int { return a + b }\n",
    );
    w(
        "calc/calc_test.go",
        r#"package calc

import (
	"os/exec"
	"testing"
)

func TestAdd(t *testing.T) {
	if Add(2, 3) != 5 {
		t.Fatal("math")
	}
	// Finds the repository, as a test that shells out to git does.
	if out, err := exec.Command("git", "rev-parse", "--is-inside-work-tree").CombinedOutput(); err != nil {
		t.Fatalf("not in a repository: %v %s", err, out)
	}
}
"#,
    );
    w("docs/notes.txt", "notes\n");
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join(".git/info/exclude"), ".quod/\n").unwrap();
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

const STATUS: &str = "git status --porcelain";
const BUILD: &str = "go build -o \"${QUOD_OUT:-.quod/out}/bin\" . && go version -m \"${QUOD_OUT:-.quod/out}/bin\" | grep -E 'vcs\\.' | sed 's/^[[:space:]]*//'";

/// The same script on the plain checkout, outside Quod (a scratch GOCACHE
/// and HOME, the binary outside the checkout so the checkout stays as it is).
fn plain(repo: &Path, script: &str) -> String {
    let scratch = tempfile::tempdir().unwrap();
    let out = Command::new("/bin/sh")
        .arg("-e")
        .arg("-c")
        .arg(script)
        .current_dir(repo)
        .env("HOME", scratch.path())
        .env("GOCACHE", scratch.path().join("gocache"))
        .env("GOFLAGS", "-mod=mod")
        .env("GOTOOLCHAIN", "local")
        .env("QUOD_OUT", scratch.path())
        .env(
            "PATH",
            format!(
                "/usr/local/go/bin:{}",
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "plain {script}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn served(env: &Env, script: &str, extra: &[&str]) -> (Out, String) {
    let o = env.run_step(script, extra);
    assert_eq!(o.code, 0, "served {script}: {}", o.stderr);
    let s = String::from_utf8(o.stdout.clone()).unwrap();
    (o, s)
}

fn vcs_lines(s: &str) -> Vec<&str> {
    s.lines()
        .filter(|l| l.starts_with("build\tvcs.") || l.starts_with("build vcs."))
        .collect()
}

/// git status and go's VCS stamp agree with the plain checkout.
fn assert_same(env: &Env, case: &str, build: &str) {
    let (_, s) = served(env, STATUS, &[]);
    let p = plain(&env.repo, STATUS);
    assert_eq!(s, p, "{case}: git status --porcelain differs");
    let (_, sb) = served(env, build, &[]);
    let pb = plain(&env.repo, build);
    assert_eq!(
        vcs_lines(&sb),
        vcs_lines(&pb),
        "{case}: go version -m vcs.* differ"
    );
    assert!(
        !vcs_lines(&pb).is_empty(),
        "{case}: no vcs stamp at all: {pb}"
    );
    println!(
        "{case}: git status --porcelain identical ({} lines); go version -m identical: {}",
        p.lines().count(),
        vcs_lines(&pb).join(", ")
    );
}

#[test]
fn served_git_index() {
    if !enabled() || !toolchain_installed(Toolchain::Go) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("vcs");
    std::fs::create_dir_all(&src).unwrap();
    fixture(&src);
    let env = Env::from_copy(&src, Some(Toolchain::Go));
    // The root is a package directory here (go stats README.md), so the
    // docs-only edit is a file outside every package.
    check(
        &env,
        BUILD,
        "docs/notes.txt",
        "calc/calc.go",
        "go test ./...",
    );

    if let Ok(spike) = std::env::var("QUOD_SPIKE_REPO") {
        let env = Env::from_copy(Path::new(&spike), Some(Toolchain::Go));
        // On a branch, as a developer's checkout is (the suite's copy is
        // detached at the pinned commit).
        git(&env.repo, &["checkout", "-q", "-B", "quod-git-index"]);
        let build = BUILD.replace(
            "go build -o \"${QUOD_OUT:-.quod/out}/bin\" .",
            "go build -o \"${QUOD_OUT:-.quod/out}/bin\" ./cmd/gh",
        );
        check(
            &env,
            &build,
            "README.md",
            "api/client.go",
            "go test ./pkg/cmdutil/...",
        );
    }
}

fn check(env: &Env, build: &str, doc: &str, src: &str, test: &str) {
    let name = env.repo.display().to_string();
    // Clean: identical, and git reads no file.
    assert_same(env, "clean", build);
    let (_, trace) = served(
        env,
        &format!("GIT_TRACE2_PERF=1 {STATUS} 2>&1 >/dev/null | grep -o 'refresh/sum_scan:[0-9]*'"),
        &[],
    );
    let scans: Vec<&str> = trace.lines().collect();
    assert!(
        !scans.is_empty() && scans.iter().all(|l| *l == "refresh/sum_scan:0"),
        "{name}: git re-checked entries: {trace}"
    );
    println!(
        "clean: git status re-checked no entry ({})",
        scans.join(" ")
    );

    // Dirty, in each way git distinguishes.
    let text = std::fs::read_to_string(env.repo.join(src)).unwrap();
    env.write(src, &format!("{text}\n// modified\n"));
    assert_same(env, "dirty: modified", build);
    env.write("untracked.txt", "new\n");
    assert_same(env, "dirty: modified + untracked", build);
    git(&env.repo, &["add", src]);
    assert_same(env, "dirty: staged + untracked", build);
    git(&env.repo, &["reset", "-q", "--hard"]);
    std::fs::remove_file(env.repo.join("untracked.txt")).unwrap();
    std::fs::remove_file(env.repo.join(doc)).unwrap();
    assert_same(env, "dirty: deleted", build);
    git(&env.repo, &["checkout", "-q", "--", doc]);
    assert_same(env, "clean again", build);

    // What replays. Record both steps on this commit (twice: the proof).
    for step in [test, build] {
        let first = env.run_step(step, &[]);
        assert_eq!(first.code, 0, "{step}: {}", first.stderr);
        let again = env.run_step(step, &[]);
        assert!(
            again.hit(),
            "{step} on the same commit must replay: {}",
            again.stderr
        );
    }
    let doc_text = std::fs::read_to_string(env.repo.join(doc)).unwrap();
    env.write(doc, &format!("{doc_text}\nedited\n"));
    env.commit("docs only");
    let t = env.run_step(test, &[]);
    assert!(
        t.hit(),
        "{test} after a {doc}-only commit must replay: {}",
        t.stderr
    );
    println!("{doc}-only commit: `{test}` REPLAYED");
    let b = env.run_step(build, &[]);
    assert!(
        b.executed() && !b.hit(),
        "go build after a new commit must run: {}",
        b.stderr
    );
    let (_, now) = served(env, build, &[]);
    assert_eq!(
        vcs_lines(&now),
        vcs_lines(&plain(&env.repo, build)),
        "after the commit"
    );
    println!("{doc}-only commit: go build EXECUTED (its binary embeds the new vcs.revision)");
    // A .go file and go.mod: both run.
    let text = std::fs::read_to_string(env.repo.join(src)).unwrap();
    env.write(src, &format!("{text}\n// edited\n"));
    env.commit("go source");
    for step in [test, build] {
        let o = env.run_step(step, &[]);
        assert!(
            o.executed() && !o.hit(),
            "{step} after a .go edit must run: {}",
            o.stderr
        );
    }
    let gomod = std::fs::read_to_string(env.repo.join("go.mod")).unwrap();
    env.write("go.mod", &format!("{gomod}\n// edited\n"));
    env.commit("go.mod");
    for step in [test, build] {
        let o = env.run_step(step, &[]);
        assert!(
            o.executed() && !o.hit(),
            "{step} after a go.mod edit must run: {}",
            o.stderr
        );
    }
    println!("{src} and go.mod commits: `{test}` and go build EXECUTED");
}
