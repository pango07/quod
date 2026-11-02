//! `.git/HEAD` from a pull request's head to its merge (DECISIONS,
//! exp/git-head-replay). On a tiny module whose pull request branch `pr`
//! and whose merge on `main` have the same tree:
//! - `go test` of a package whose tests run no git replays on the merge:
//!   cmd/go itself reads no `.git` for go test (Go 1.27.3: setBuildInfo
//!   wants VCS information only for go build and go install, and never for
//!   a test binary unless -buildvcs=true);
//! - `go test` of a package whose test only finds the repository (`git
//!   rev-parse --show-toplevel`, as cli/cli's tests do) misses on the merge,
//!   on `.git/HEAD` and on nothing else, and its receipt says what that
//!   means: the root cause of 0e's F2, reproduced;
//! - tests that print the branch and commit (through git, and by reading
//!   `.git/HEAD` as a file) never replay across the branch change, and
//!   their output under Quod is the plain checkout's;
//! - each job replays its own branch's record (a re-run on `main`, a new
//!   push to `pr` with the same commit);
//! - A7 on `.git`: every entry the merge's manifests recorded, mutated
//!   (the other branch's HEAD, a detached HEAD, every file's first byte,
//!   a new name in every listing, every absent path created), matches none
//!   of them.
//!
//! Gated on QUOD_INTEGRATION=1.

mod common;
use std::path::Path;
use std::process::Command;

use common::*;
use quod_cas::{Cas, Entry, Hash, Kind, Tree, child_path};
use quod_key::Toolchain;
use quod_ledger::Ledger;
use quod_manifest::{Manifest, manifest_id, matches};

const CALC: &str = "go test ./calc/";
const DISCO: &str = "go test ./disco/";
const BRANCH: &str = "go test -v -run TestBranch ./branch/";
const HEADFILE: &str = "go test -v -run TestHeadFile ./branch/";

fn fixture(dir: &Path) {
    let w = |p: &str, s: &str| {
        let p = dir.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    };
    w("go.mod", "module example.com/head\n\ngo 1.27\n");
    w("feature.txt", "v0\n");
    w(
        "calc/calc.go",
        "package calc\n\nfunc Add(a, b int) int { return a + b }\n",
    );
    w(
        "calc/calc_test.go",
        "package calc\n\nimport \"testing\"\n\nfunc TestAdd(t *testing.T) {\n\tif Add(2, 3) != 5 {\n\t\tt.Fatal(\"math\")\n\t}\n}\n",
    );
    // Finds the repository and nothing more, as cli/cli's tests do (its
    // git client's ToplevelDir, its credential tests' git config).
    w(
        "disco/disco_test.go",
        r#"package disco

import (
	"os/exec"
	"testing"
)

func TestToplevel(t *testing.T) {
	if out, err := exec.Command("git", "rev-parse", "--show-toplevel").CombinedOutput(); err != nil {
		t.Fatalf("not in a repository: %v %s", err, out)
	}
}
"#,
    );
    // Prints the branch and the commit: through git, and by reading the
    // file the way a program that is not git would.
    w(
        "branch/branch_test.go",
        r#"package branch

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

func git(t *testing.T, args ...string) string {
	out, err := exec.Command("git", args...).Output()
	if err != nil {
		t.Fatalf("git %v: %v", args, err)
	}
	return strings.TrimSpace(string(out))
}

func TestBranch(t *testing.T) {
	fmt.Printf("branch=%s\n", git(t, "branch", "--show-current"))
	fmt.Printf("commit=%s\n", git(t, "rev-parse", "HEAD"))
}

func TestHeadFile(t *testing.T) {
	b, err := os.ReadFile(filepath.Join(git(t, "rev-parse", "--show-toplevel"), ".git", "HEAD"))
	if err != nil {
		t.Fatal(err)
	}
	fmt.Printf("head=%s", b)
}
"#,
    );
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join(".git/info/exclude"), ".quod/\n").unwrap();
}

/// `<cmd> -q -m <msg> [<arg>]` as a fixed author.
fn commit(dir: &Path, msg: &str, cmd: &[&str], arg: Option<&str>) {
    let mut args = vec!["-c", "user.email=t@t", "-c", "user.name=t"];
    args.extend_from_slice(cmd);
    args.extend(["-q", "-m", msg]);
    args.extend(arg);
    git(dir, &args);
}

fn rev(dir: &Path, r: &str) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", r])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// The same script on the plain checkout, outside Quod.
fn plain(repo: &Path, script: &str) -> String {
    let scratch = tempfile::tempdir().unwrap();
    let out = Command::new("/bin/sh")
        .arg("-e")
        .arg("-c")
        .arg(script)
        .current_dir(repo)
        .env("HOME", scratch.path())
        .env("GOCACHE", scratch.path().join("gocache"))
        .env("GOFLAGS", "-mod=mod -count=1")
        .env("GOTOOLCHAIN", "local")
        .env(
            "PATH",
            format!(
                "/usr/local/go/bin:{}",
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .output()
        .unwrap();
    assert!(out.status.success(), "plain {script}");
    String::from_utf8(out.stdout).unwrap()
}

/// The lines a test printed (`key=value`), without go test's own.
fn printed(stdout: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .filter(|l| l.starts_with("branch=") || l.starts_with("commit=") || l.starts_with("head="))
        .map(String::from)
        .collect()
}

/// The last `run` event's field.
fn run_field(env: &Env, f: &str) -> serde_json::Value {
    let e = last_event(&env.state(), "run").expect("a run event");
    e.get(f)
        .or_else(|| e.get("fields").and_then(|x| x.get(f)))
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

/// The tree `quod run` serves: the checkout without `.git` and `.quod/`,
/// the profile's dependency trees from the deps dir, and the served
/// `.git` with its index refreshed (as acceptance.rs's A7 builds it).
fn workspace_tree(cas: &Cas, env: &Env, deps: &Path) -> Tree {
    let mut tree = cas
        .import_dir(&env.repo)
        .unwrap()
        .without_top_level(".git")
        .without_top_level(".quod");
    for name in Toolchain::Go
        .profile()
        .seed_dirs
        .iter()
        .filter_map(|d| d.strip_prefix(".quod/"))
    {
        let dir = deps.join(name);
        if dir.is_dir() {
            tree = tree.merged_at(&format!(".quod/{name}"), &cas.import_dir(&dir).unwrap());
        }
    }
    let out = env.quod(&["__served-git", "--repo", &env.repo.to_string_lossy()]);
    assert_eq!(out.code, 0, "__served-git: {}", out.stderr);
    let dot: Vec<Entry> = serde_json::from_slice(&out.stdout).unwrap();
    tree = tree.merged_at(".git", &Tree::from_leaves(dot));
    quod_fs::git::refresh_index(&tree, cas, 1001, 1001)
        .unwrap()
        .0
}

#[test]
fn pr_head_to_merge() {
    if !enabled() || !toolchain_installed(Toolchain::Go) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("head");
    std::fs::create_dir_all(&src).unwrap();
    fixture(&src);
    let env = Env::from_copy(&src, Some(Toolchain::Go));
    let repo = &env.repo;
    git(repo, &["add", "-A"]);
    commit(repo, "base", &["commit"], None);
    // The pull request: one commit on `pr`; the merge: a merge commit on
    // `main`, which has not moved, so its tree is the pull request's.
    git(repo, &["checkout", "-q", "-b", "pr"]);
    env.write("feature.txt", "v1\n");
    git(repo, &["add", "-A"]);
    commit(repo, "feature", &["commit"], None);
    let head = rev(repo, "pr");
    git(repo, &["checkout", "-q", "main"]);
    commit(
        repo,
        "Merge pull request #1",
        &["merge", "--no-ff"],
        Some("pr"),
    );
    let merge = rev(repo, "main");
    assert_ne!(head, merge);
    assert_eq!(
        rev(repo, "pr^{tree}"),
        rev(repo, "main^{tree}"),
        "same tree"
    );

    let deps_probe = env.run_step("true", &["--allow-net"]);
    assert_eq!(deps_probe.code, 0, "{}", deps_probe.stderr);
    let deps: std::path::PathBuf = deps_probe
        .stderr
        .lines()
        .find_map(|l| l.trim_start().strip_prefix("caches in "))
        .map(std::path::PathBuf::from)
        .expect("the network step's banner names its caches dir");

    // The pull request's job, on `pr`.
    git(repo, &["checkout", "-q", "pr"]);
    let mut pr_out = Vec::new();
    for s in [CALC, DISCO, BRANCH, HEADFILE] {
        let o = env.run_step(s, &[]);
        assert!(o.executed() && o.code == 0, "pr {s}: {}", o.stderr);
        pr_out.push(o.stdout);
    }
    assert_eq!(
        printed(&pr_out[2]),
        vec!["branch=pr".to_string(), format!("commit={head}")]
    );
    assert_eq!(printed(&pr_out[3]), vec!["head=ref: refs/heads/pr"]);

    // The merge's job, on `main`.
    git(repo, &["checkout", "-q", "main"]);
    let calc = env.run_step(CALC, &[]);
    assert!(
        calc.hit(),
        "go test with no git in its tests must replay from the pull request to its merge: {}",
        calc.stderr
    );
    println!("merge: `{CALC}` REPLAYED (cmd/go reads no .git for go test)");

    let disco = env.run_step(DISCO, &[]);
    assert!(
        disco.executed() && !disco.hit() && disco.code == 0,
        "{}",
        disco.stderr
    );
    assert_eq!(
        run_field(&env, "first_mismatch"),
        ".git/HEAD",
        "{}",
        disco.stderr
    );
    assert_eq!(
        run_field(&env, "mismatches"),
        1,
        "only HEAD differs: {}",
        disco.stderr
    );
    assert!(
        disco.stderr.contains("differed at .git/HEAD")
            && disco
                .stderr
                .contains("the step read .git/HEAD: a git command it ran found the repository")
            && !disco.stderr.contains("-buildvcs"),
        "{}",
        disco.stderr
    );
    println!(
        "merge: `{DISCO}` EXECUTED, its only changed input .git/HEAD (ref: refs/heads/pr -> ref: refs/heads/main): 0e's F2"
    );

    for (s, want) in [
        (
            BRANCH,
            vec!["branch=main".to_string(), format!("commit={merge}")],
        ),
        (HEADFILE, vec!["head=ref: refs/heads/main".to_string()]),
    ] {
        let o = env.run_step(s, &[]);
        assert!(
            o.executed() && !o.hit() && o.code == 0,
            "a test that prints the branch or commit must not replay across them: {}",
            o.stderr
        );
        assert_eq!(printed(&o.stdout), want, "{s}");
        assert_eq!(
            printed(&o.stdout),
            printed(plain(repo, s).as_bytes()),
            "{s} vs plain"
        );
        println!("merge: `{s}` EXECUTED: {}", want.join(" "));
    }

    // A re-run of the merge job replays its own record; so does the pull
    // request's job, back on `pr`.
    for s in [DISCO, BRANCH, HEADFILE] {
        let o = env.run_step(s, &[]);
        assert!(o.hit(), "re-run on main: {s}: {}", o.stderr);
        assert_eq!(
            printed(&o.stdout),
            printed(plain(repo, s).as_bytes()),
            "{s}"
        );
    }
    git(repo, &["checkout", "-q", "pr"]);
    for s in [DISCO, BRANCH, HEADFILE] {
        let o = env.run_step(s, &[]);
        assert!(o.hit(), "re-run on pr: {s}: {}", o.stderr);
        assert_eq!(
            printed(&o.stdout),
            printed(plain(repo, s).as_bytes()),
            "{s}"
        );
    }
    // Another branch at the merge commit, and the merge commit detached:
    // a new HEAD each, so the git readers run.
    for (how, args) in [
        (
            "branch release at the merge",
            vec!["checkout", "-q", "-B", "release", "main"],
        ),
        (
            "detached at the merge",
            vec!["checkout", "-q", "--detach", "main"],
        ),
    ] {
        git(repo, &args);
        for s in [DISCO, BRANCH, HEADFILE] {
            let o = env.run_step(s, &[]);
            assert!(o.executed() && !o.hit(), "{how}: {s}: {}", o.stderr);
            if s != DISCO {
                assert_eq!(
                    printed(&o.stdout),
                    printed(plain(repo, s).as_bytes()),
                    "{how}: {s}"
                );
            }
        }
    }
    println!("re-runs on main and pr REPLAYED; a new branch and a detached HEAD EXECUTED");

    // A7 on `.git`: the merge's manifests, every recorded entry mutated.
    git(repo, &["checkout", "-q", "main"]);
    let cas = Cas::open(env.state().join("cas")).unwrap();
    let tree = workspace_tree(&cas, &env, &deps);
    let ledger = Ledger::open(&env.state().join("ledger.sqlite")).unwrap();
    let mut checked = 0usize;
    for s in [DISCO, BRANCH, HEADFILE] {
        let o = env.run_step(s, &[]);
        assert!(o.hit(), "{s} on main: {}", o.stderr);
        let key: Hash = run_field(&env, "step_key")
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let mid = run_field(&env, "manifest_id");
        let m: Manifest = ledger
            .candidates(&key, quod_ledger::CANDIDATES)
            .unwrap()
            .into_iter()
            .find(|m| manifest_id(m).unwrap().to_string() == mid.as_str().unwrap())
            .expect("the replayed manifest");
        matches(&m, &tree)
            .unwrap_or_else(|mm| panic!("{s}: the tree must match before mutating: {mm}"));
        let git_entries: Vec<&Entry> = m
            .entries
            .iter()
            .filter(|e| e.p.starts_with(".git"))
            .collect();
        assert!(
            git_entries.iter().any(|e| e.p == ".git/HEAD"),
            "{s} read .git/HEAD"
        );
        let mut mutated: Vec<(String, Tree)> = Vec::new();
        // The other branch's HEAD, a detached HEAD at either commit.
        for content in [
            "ref: refs/heads/pr\n".to_string(),
            format!("{merge}\n"),
            format!("{head}\n"),
        ] {
            mutated.push((
                format!(".git/HEAD = {content:?}"),
                tree.with_leaf_changed(Entry::file(
                    ".git/HEAD",
                    Hash::of(content.as_bytes()),
                    0o644,
                )),
            ));
        }
        for e in &git_entries {
            let t = match e.k {
                Kind::File => {
                    let mut bytes = cas.get(&e.h.unwrap()).unwrap().unwrap();
                    match bytes.first_mut() {
                        Some(b) => *b ^= 0x01,
                        None => bytes.push(b'x'),
                    }
                    tree.with_leaf_changed(Entry::file(e.p.clone(), Hash::of(&bytes), e.m.unwrap()))
                }
                Kind::Dir => tree.with_outputs(&[Entry::file(
                    child_path(&e.p, "zz_quod_a7"),
                    Hash::of(b"x"),
                    0o644,
                )]),
                Kind::Absent => {
                    tree.with_outputs(&[Entry::file(e.p.clone(), Hash::of(b"x"), 0o644)])
                }
                Kind::Symlink => tree.with_leaf_changed(Entry::symlink(e.p.clone(), "zz_quod_a7")),
                Kind::Deleted => unreachable!("manifests never hold x"),
            };
            mutated.push((format!("{} {:?}", e.p, e.k), t));
        }
        let hits: Vec<&String> = mutated
            .iter()
            .filter(|(_, t)| matches(&m, t).is_ok())
            .map(|(what, _)| what)
            .collect();
        assert!(hits.is_empty(), "{s}: false hits on .git: {hits:?}");
        checked += mutated.len();
        println!(
            "A7 on .git, `{s}`: {} recorded .git entries, {} mutations, 0 false hits",
            git_entries.len(),
            mutated.len()
        );
    }
    assert!(checked > 0);

    // The same with Go's test cache under Quod's guard (`--warm`, as 0e's
    // traffic ran): a test that starts a process is never cached by go
    // test there, so the repository-finding test runs, and reads HEAD, on
    // every execution.
    git(repo, &["checkout", "-q", "pr"]);
    for s in [CALC, DISCO] {
        let o = env.run_step(s, &["--warm"]);
        assert!(o.code == 0, "warm pr {s}: {}", o.stderr);
    }
    git(repo, &["checkout", "-q", "main"]);
    let calc = env.run_step(CALC, &["--warm"]);
    assert!(calc.hit(), "warm merge {CALC}: {}", calc.stderr);
    let disco = env.run_step(DISCO, &["--warm"]);
    assert!(disco.executed() && !disco.hit(), "{}", disco.stderr);
    assert_eq!(run_field(&env, "first_mismatch"), ".git/HEAD");
    assert_eq!(run_field(&env, "mismatches"), 1);
    println!("warm: merge `{CALC}` REPLAYED, `{DISCO}` EXECUTED on .git/HEAD alone");
}
