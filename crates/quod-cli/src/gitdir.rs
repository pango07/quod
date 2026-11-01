//! The `.git` a `quod run` step is served: what a plain checkout of the
//! same commit shows a build, so go build stamps the same VCS information
//! (vcs.revision, vcs.time, vcs.modified) and `git status --porcelain`
//! reports the same changes as in the user's checkout, without the step
//! seeing the user's own `.git` (its config may hold credentials, its
//! history is unbounded, and nothing a step does may reach it).
//!
//! It is made of:
//! - the commit's objects at depth 1 (a shallow fetch from the checkout into
//!   a scratch repository, imported once per commit and memoized at
//!   `<state>/gitdirs/<sha>.v2.json`), with a fixed config (no remote, no
//!   user settings) and no reflogs;
//! - HEAD as the checkout has it: `ref: refs/heads/<branch>` with that
//!   branch at the commit, or the commit itself when detached;
//! - the checkout's index (so staged changes stay staged), which the caller
//!   refreshes against the served tree (`quod_fs::git::refresh_index`):
//!   entries whose file is unchanged get the served stat data, so git reads
//!   none of them, and the others are cleared, so git looks at exactly
//!   those, as it would on the user's disk;
//! - the checkout's `info/exclude`, plus `/.quod/` (Quod's own state at the
//!   top of the workspace is never an untracked file).
//!
//! Known differences from the user's `.git`, recorded in DECISIONS.md: one
//! commit of history (a step that walks history sees a shallow clone, as
//! on GitHub's runner with actions/checkout's default depth), no remotes
//! and none of the user's git config.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, bail};
use quod_cas::{Cas, Entry, Tree};

/// The served `.git`, as a tree rooted at the `.git` directory itself, or
/// `None` when `repo` is not a git checkout with a commit checked out (then
/// no `.git` is served, as before). An error for a checkout this cannot
/// serve faithfully (`graft` then serves no `.git` and says why): `repo`
/// below the checkout's top level (the index's paths are the top level's),
/// an initialized submodule (its `.git` file points into `.git/modules/`,
/// which is not served, and `git status` would fail where it works on
/// disk), a repository whose objects are not SHA-1, a fetch that fails.
pub fn served_git(cas: &Cas, state_dir: &Path, repo: &Path) -> anyhow::Result<Option<Tree>> {
    let Some(sha) = git_out(repo, &["rev-parse", "--verify", "-q", "HEAD^{commit}"]) else {
        return Ok(None);
    };
    let top = git_out(repo, &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .context("the checkout's top level")?;
    if top.canonicalize()? != repo.canonicalize()? {
        bail!(
            "{} is below the checkout's top level {}",
            repo.display(),
            top.display()
        );
    }
    if let Some(f) = git_out(repo, &["rev-parse", "--show-object-format"])
        && f != "sha1"
    {
        bail!("the repository's objects are {f}, not sha1");
    }
    if let Some(sub) = populated_gitlink(repo) {
        bail!("{sub} is an initialized submodule (its .git/modules is not served)");
    }
    let git_dir = PathBuf::from(
        git_out(repo, &["rev-parse", "--absolute-git-dir"]).context("the checkout's git dir")?,
    );
    let common = git_out(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .map(PathBuf::from)
    .unwrap_or_else(|| git_dir.clone());
    let head = git_out(repo, &["symbolic-ref", "-q", "HEAD"]);
    let base = objects(cas, state_dir, repo, &sha)?;

    // Directories too: git finds a repository only where refs/ and
    // objects/ exist, empty or not.
    let mut leaves: Vec<Entry> = base
        .iter()
        .filter(|e| e.p != "HEAD" && e.p != "index" && !e.p.starts_with("refs/heads/"))
        .cloned()
        .collect();
    leaves.push(Entry::dir("refs/heads", quod_cas::Hash::of(b"")));
    let mut put = |p: &str, bytes: &[u8]| -> anyhow::Result<()> {
        leaves.push(Entry::file(p, cas.put(bytes)?, 0o644));
        Ok(())
    };
    match &head {
        Some(r) => {
            put("HEAD", format!("ref: {r}\n").as_bytes())?;
            if r.starts_with("refs/heads/") && r.split('/').all(quod_cas::valid_name) {
                put(r, format!("{sha}\n").as_bytes())?;
            }
        }
        None => put("HEAD", format!("{sha}\n").as_bytes())?,
    }
    // The index git uses (a worktree's own), and a split index's shared
    // part (git reads it by name; the refresh leaves those entries alone).
    let index = git_out(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )
    .map(PathBuf::from)
    .unwrap_or_else(|| git_dir.join("index"));
    if let Ok(bytes) = std::fs::read(&index) {
        put("index", &bytes)?;
    }
    if let Ok(dir) = std::fs::read_dir(&git_dir) {
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("sharedindex.")
                && let Ok(bytes) = std::fs::read(e.path())
            {
                put(&name, &bytes)?;
            }
        }
    }
    let mut exclude = std::fs::read_to_string(common.join("info/exclude")).unwrap_or_default();
    if !exclude.lines().any(|l| l.trim() == "/.quod/") {
        if !exclude.is_empty() && !exclude.ends_with('\n') {
            exclude.push('\n');
        }
        exclude.push_str("/.quod/\n");
    }
    put("info/exclude", exclude.as_bytes())?;
    Ok(Some(Tree::from_leaves(leaves)))
}

/// The commit's objects and the fixed parts of the `.git` (config,
/// shallow, the empty refs), from the memo or a shallow fetch.
fn objects(cas: &Cas, state_dir: &Path, repo: &Path, sha: &str) -> anyhow::Result<Tree> {
    let memo = state_dir.join("gitdirs").join(format!("{sha}.v2.json"));
    if let Ok(bytes) = std::fs::read(&memo)
        && let Ok(entries) = serde_json::from_slice::<Vec<Entry>>(&bytes)
        // Files only: a directory's hash is its listing, never a blob.
        && entries
            .iter()
            .filter(|e| e.k == quod_cas::Kind::File)
            .all(|e| e.h.is_some_and(|h| cas.has(&h)))
    {
        return Ok(Tree::from_leaves(entries));
    }
    let scratch = state_dir
        .join("tmp")
        .join(format!("gitdir-{}", ulid::Ulid::generate()));
    std::fs::create_dir_all(&scratch)?;
    let result = (|| -> anyhow::Result<Tree> {
        let run = |args: &[&str]| -> anyhow::Result<()> {
            let st = Command::new("git")
                .arg("-C")
                .arg(&scratch)
                .args(args)
                .env("GIT_TERMINAL_PROMPT", "0")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .status()
                .with_context(|| format!("git {}", args[0]))?;
            if !st.success() {
                bail!("git {args:?} failed building the served .git");
            }
            Ok(())
        };
        run(&["init", "-q", "--template="])?;
        run(&["config", "core.logAllRefUpdates", "false"])?;
        run(&["config", "gc.auto", "0"])?;
        let url = format!("file://{}", repo.display());
        // A pack, never loose objects (see quod-hooks' checkout). The local
        // upload-pack inherits `-c`: HEAD's commit by id, advertised or not
        // (a detached HEAD's, a commit no ref names); one pack-objects
        // thread, so the pack's bytes and name are the same for a commit
        // in every state directory (a step reading objects, go build's git
        // log, then matches across them).
        run(&[
            "-c",
            "fetch.unpackLimit=1",
            "-c",
            "uploadpack.allowAnySHA1InWant=true",
            "-c",
            "pack.threads=1",
            "fetch",
            "-q",
            "--no-tags",
            "--depth=1",
            "--no-write-fetch-head",
            &url,
            sha,
        ])?;
        let dot = scratch.join(".git");
        for leftover in ["FETCH_HEAD", "logs", "ORIG_HEAD"] {
            let p = dot.join(leftover);
            if p.exists() {
                quod_cas::remove_any(&p)?;
            }
        }
        Ok(cas.import_dir(&dot)?)
    })();
    let _ = quod_cas::remove_any(&scratch);
    let tree = result?;
    std::fs::create_dir_all(memo.parent().unwrap())?;
    let tmp = memo.with_extension(format!("json.tmp.{}", std::process::id()));
    let entries: Vec<&Entry> = tree.iter().collect();
    std::fs::write(&tmp, serde_json::to_vec(&entries)?)?;
    std::fs::rename(&tmp, &memo)?;
    Ok(tree)
}

/// The first gitlink in the index whose work tree is checked out (holds a
/// `.git`): an initialized submodule.
fn populated_gitlink(repo: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["ls-files", "--stage", "-z"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.stdout
        .split(|&b| b == 0)
        .filter_map(|rec| std::str::from_utf8(rec).ok())
        .filter_map(|rec| rec.split_once('\t'))
        .filter(|(meta, _)| meta.starts_with("160000 "))
        .map(|(_, path)| path)
        .find(|path| repo.join(path).join(".git").exists())
        .map(String::from)
}

/// `git -C repo <args>`'s trimmed stdout, if it succeeded with output.
fn git_out(repo: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

/// `tree` (a workspace without `.git`) with the served `.git` grafted in
/// and its index refreshed for a step running as `uid`:`gid`.
pub fn graft(
    cas: &Cas,
    state_dir: &Path,
    repo: &Path,
    tree: Tree,
    uid: u32,
    gid: u32,
) -> anyhow::Result<Tree> {
    // A checkout this cannot serve gets no .git, as before this was served
    // at all: the step runs, and says why.
    let dot = match served_git(cas, state_dir, repo) {
        Ok(Some(dot)) => dot,
        Ok(None) => return Ok(tree),
        Err(e) => {
            eprintln!("QUOD  note: no .git served: {e:#}");
            tracing::warn!(target: "quod", error = %format!("{e:#}"), "no .git served");
            return Ok(tree);
        }
    };
    let tree = tree.merged_at(".git", &dot);
    let (tree, r) =
        quod_fs::git::refresh_index(&tree, cas, uid, gid).context("refresh the git index")?;
    tracing::info!(target: "quod", refresh = ?r, "git index");
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let st = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.email=t@t", "-c", "user.name=t"])
            .args(args)
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(st.success(), "git {args:?}");
    }

    #[test]
    fn served_git_follows_the_checkout() {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("no git; skipping");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let state = tmp.path().join("state");
        let cas = Cas::open(state.join("cas")).unwrap();
        // Not a repository, or no commit yet: no .git.
        assert!(served_git(&cas, &state, &repo).unwrap().is_none());
        git(&repo, &["init", "-q", "-b", "main"]);
        assert!(served_git(&cas, &state, &repo).unwrap().is_none());
        std::fs::write(repo.join("a"), "a\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "one"]);
        let sha = git_out(&repo, &["rev-parse", "HEAD"]).unwrap();
        let file = |t: &Tree, p: &str| {
            String::from_utf8(cas.get(&t.get(p).unwrap().h.unwrap()).unwrap().unwrap()).unwrap()
        };
        let t = served_git(&cas, &state, &repo).unwrap().unwrap();
        assert_eq!(file(&t, "HEAD"), "ref: refs/heads/main\n");
        assert_eq!(file(&t, "refs/heads/main"), format!("{sha}\n"));
        assert_eq!(file(&t, "shallow"), format!("{sha}\n"));
        assert!(file(&t, "info/exclude").ends_with("/.quod/\n"));
        assert!(!file(&t, "config").contains("[remote"));
        assert!(t.get("index").is_some() && t.get("FETCH_HEAD").is_none());
        for d in ["refs", "refs/tags", "objects/pack"] {
            assert_eq!(t.get(d).map(|e| e.k), Some(quod_cas::Kind::Dir), "{d}");
        }
        assert!(state.join(format!("gitdirs/{sha}.v2.json")).is_file());
        // Detached: HEAD is the commit, no branch.
        git(&repo, &["checkout", "-q", "--detach"]);
        let d = served_git(&cas, &state, &repo).unwrap().unwrap();
        assert_eq!(file(&d, "HEAD"), format!("{sha}\n"));
        assert!(d.get("refs/heads/main").is_none());
        assert_eq!(d.get("refs/heads").map(|e| e.k), Some(quod_cas::Kind::Dir));
    }

    fn committed(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q", "-b", "main"]);
        for i in 0..150 {
            std::fs::write(dir.join(format!("f{i}")), format!("{i}\n")).unwrap();
        }
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", "one"]);
    }

    #[test]
    fn the_objects_memo_is_reused_and_packs_do_not_depend_on_the_fetch() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        committed(&repo);
        let pack = |state: &Path| -> Vec<Entry> {
            let cas = Cas::open(state.join("cas")).unwrap();
            let t = served_git(&cas, state, &repo).unwrap().unwrap();
            t.iter()
                .filter(|e| e.p.starts_with("objects/pack/"))
                .cloned()
                .collect()
        };
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        let first = pack(&a);
        assert!(first.iter().any(|e| e.p.ends_with(".pack")), "{first:?}");
        // The same commit fetched into another state: the same pack.
        assert_eq!(pack(&b), first);
        // A second call in one state uses the memo: with no room to fetch
        // (tmp/ is a file now), it still serves, the same objects.
        let _ = quod_cas::remove_any(&a.join("tmp"));
        std::fs::write(a.join("tmp"), "not a directory").unwrap();
        assert_eq!(pack(&a), first);
    }

    #[test]
    fn what_it_cannot_serve_gets_no_git_and_the_step_still_runs() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        let cas = Cas::open(state.join("cas")).unwrap();
        let ws = || Tree::from_leaves([Entry::file("a", cas.put(b"a\n").unwrap(), 0o644)]);
        let grafted = |repo: &Path, state: &Path| {
            graft(&cas, state, repo, ws(), 1001, 1001)
                .unwrap()
                .get(".git")
                .is_some()
        };
        // An initialized submodule: its .git file points into the
        // superproject's .git/modules, which is not served.
        let sub = tmp.path().join("sub");
        committed(&sub);
        let sup = tmp.path().join("sup");
        committed(&sup);
        git(
            &sup,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "-q",
                sub.to_str().unwrap(),
                "vendored",
            ],
        );
        git(&sup, &["commit", "-q", "-m", "submodule"]);
        let e = served_git(&cas, &state, &sup).unwrap_err().to_string();
        assert!(e.contains("vendored") && e.contains("submodule"), "{e}");
        assert!(!grafted(&sup, &state));
        // Not initialized (a fresh clone without --recurse-submodules): an
        // empty directory git status handles, served as usual.
        let clone = tmp.path().join("clone");
        git(
            tmp.path(),
            &[
                "clone",
                "-q",
                sup.to_str().unwrap(),
                clone.to_str().unwrap(),
            ],
        );
        assert!(grafted(&clone, &state));
        // --repo below the top level: the index's paths are the top level's.
        std::fs::create_dir_all(sup.join("deep")).unwrap();
        let e = served_git(&cas, &state, &sup.join("deep"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("below the checkout's top level"), "{e}");
        assert!(!grafted(&sup.join("deep"), &state));
        // A fetch that cannot happen (no room for its scratch repository):
        // no .git, not a failed run.
        let blocked = tmp.path().join("blocked");
        std::fs::create_dir_all(&blocked).unwrap();
        std::fs::write(blocked.join("tmp"), "not a directory").unwrap();
        assert!(!grafted(&clone, &blocked));
    }
}
