//! `actions/checkout` from the CAS: the commit's tree is fetched once into a
//! bare mirror, imported into the store, and from then on served, never
//! copied. A repeat of the same commit costs a memo read.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{Context, bail};
use sp_cas::{Cas, Entry, Tree};

use crate::step::CheckoutInputs;

pub struct CheckedOut {
    pub sha: String,
    pub tree: Tree,
    /// The tree came from the memo, not from git.
    pub memo: bool,
    pub elapsed: std::time::Duration,
    /// actions/checkout's `ref` output: the ref as given, GITHUB_REF for
    /// the workflow's own commit, empty for a sha.
    pub ref_output: String,
}

/// The commit `inputs` name, as a tree in the store. Before anything is
/// served, from the memo or a fetch, the job's token must be able to read
/// the repository (an `ls-remote` with it), and the ref resolves exactly:
/// a full sha as is, `refs/...` by that name, anything else as
/// `refs/heads/<ref>` then `refs/tags/<ref>`.
pub fn checkout(
    state_dir: &Path,
    cas: &Cas,
    inputs: &CheckoutInputs,
) -> anyhow::Result<CheckedOut> {
    let t = Instant::now();
    let url = format!(
        "{}/{}",
        inputs.server_url.trim_end_matches('/'),
        inputs.repository
    );
    let mirror = mirror_dir(state_dir, &inputs.repository.to_ascii_lowercase());
    ensure_mirror(&mirror)?;
    let auth = Auth::new(&inputs.server_url, inputs.token.as_deref());
    let (sha, ref_output) = resolve(inputs, &url, &mirror, &auth)?;
    // v2: the tree has a .git.
    let memo = state_dir.join("trees").join(format!("{sha}.v2.json"));
    if let Ok(bytes) = std::fs::read(&memo)
        && let Ok(tree) = tree_from_json(&bytes)
    {
        return Ok(CheckedOut {
            sha,
            tree,
            memo: true,
            elapsed: t.elapsed(),
            ref_output,
        });
    }
    if !has_commit(&mirror, &sha) {
        fetch(&mirror, &url, &sha, &auth)?;
        if !has_commit(&mirror, &sha) {
            bail!("{url}: commit {sha} not found after fetch");
        }
    }
    let tree = import_commit(cas, &mirror, &sha, &url, state_dir)?;
    std::fs::create_dir_all(memo.parent().unwrap())?;
    let tmp = memo.with_extension("json.tmp");
    std::fs::write(&tmp, tree_to_json(&tree)?)?;
    std::fs::rename(&tmp, &memo)?;
    Ok(CheckedOut {
        sha,
        tree,
        memo: false,
        elapsed: t.elapsed(),
        ref_output,
    })
}

fn mirror_dir(state_dir: &Path, repository: &str) -> PathBuf {
    let safe: String = repository
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let safe = safe.replace("..", "__");
    state_dir.join("mirrors").join(format!("{safe}.git"))
}

/// How git authenticates: the token as actions/checkout sends it (basic
/// auth, `x-access-token`), through git's environment (GIT_CONFIG_COUNT),
/// never its command line, and only for the server.
struct Auth {
    env: Vec<(String, String)>,
}

impl Auth {
    fn new(server_url: &str, token: Option<&str>) -> Auth {
        let mut env = vec![("GIT_TERMINAL_PROMPT".to_string(), "0".to_string())];
        if let Some(t) = token {
            let basic = base64(format!("x-access-token:{t}").as_bytes());
            env.extend([
                ("GIT_CONFIG_COUNT".to_string(), "1".to_string()),
                (
                    "GIT_CONFIG_KEY_0".to_string(),
                    format!("http.{}/.extraheader", server_url.trim_end_matches('/')),
                ),
                (
                    "GIT_CONFIG_VALUE_0".to_string(),
                    format!("AUTHORIZATION: basic {basic}"),
                ),
            ]);
        }
        Auth { env }
    }
}

/// The commit and the `ref` output.
fn resolve(
    inputs: &CheckoutInputs,
    url: &str,
    mirror: &Path,
    auth: &Auth,
) -> anyhow::Result<(String, String)> {
    let r = inputs.reference.trim();
    let own = inputs
        .repository
        .eq_ignore_ascii_case(&inputs.github_repository);
    let wanted: Vec<String> = if r.is_empty() {
        vec!["HEAD".into()]
    } else if is_sha(r) {
        // Nothing to resolve; the listing is the authorization.
        vec!["HEAD".into()]
    } else if r.starts_with("refs/") {
        vec![r.to_string(), format!("{r}^{{}}")]
    } else {
        vec![
            format!("refs/heads/{r}"),
            format!("refs/tags/{r}"),
            format!("refs/tags/{r}^{{}}"),
        ]
    };
    let refs = ls_remote(url, &wanted, mirror, auth)?;
    let exact = |name: &str| {
        refs.iter()
            .find(|(n, _)| n == name)
            .map(|(_, sha)| sha.clone())
    };
    if r.is_empty() {
        if own && is_sha(&inputs.github_sha) {
            return Ok((
                inputs.github_sha.to_ascii_lowercase(),
                inputs.github_ref.clone(),
            ));
        }
        let sha = exact("HEAD").with_context(|| format!("{url}: no HEAD"))?;
        let target = refs
            .iter()
            .find(|(n, _)| n == "symref:HEAD")
            .map(|(_, t)| t.clone())
            .unwrap_or_default();
        return Ok((sha, target));
    }
    if is_sha(r) {
        return Ok((r.to_ascii_lowercase(), String::new()));
    }
    let candidates: Vec<String> = if r.starts_with("refs/") {
        vec![r.to_string()]
    } else {
        vec![format!("refs/heads/{r}"), format!("refs/tags/{r}")]
    };
    for c in candidates {
        // A tag's peeled commit, else the ref itself.
        if let Some(sha) = exact(&format!("{c}^{{}}")).or_else(|| exact(&c)) {
            return Ok((sha, r.to_string()));
        }
    }
    bail!("{url}: no branch or tag named {r:?}")
}

fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn git(mirror: &Path, auth: Option<&Auth>) -> Command {
    let mut c = Command::new("git");
    c.arg("-C").arg(mirror);
    c.env("GIT_TERMINAL_PROMPT", "0");
    if let Some(a) = auth {
        c.envs(a.env.iter().map(|(k, v)| (k, v)));
    }
    c.stdin(Stdio::null());
    c
}

/// `(name, sha)` for every advertised ref matching `patterns` exactly, plus
/// `("symref:HEAD", target)`. Fails when the token cannot read the
/// repository: the authorization every checkout passes.
fn ls_remote(
    url: &str,
    patterns: &[String],
    mirror: &Path,
    auth: &Auth,
) -> anyhow::Result<Vec<(String, String)>> {
    let out = git(mirror, Some(auth))
        .args(["ls-remote", "--symref", url])
        .args(patterns)
        .stderr(Stdio::piped())
        .output()
        .context("git ls-remote")?;
    if !out.status.success() {
        bail!(
            "{url}: this job's token cannot read the repository (git ls-remote: {})",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let mut refs = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Some((left, name)) = line.split_once('\t') else {
            continue;
        };
        match left.strip_prefix("ref: ") {
            Some(target) => refs.push((format!("symref:{name}"), target.to_string())),
            None => refs.push((name.to_string(), left.to_ascii_lowercase())),
        }
    }
    Ok(refs)
}

fn ensure_mirror(mirror: &Path) -> anyhow::Result<()> {
    if mirror.join("HEAD").is_file() {
        return Ok(());
    }
    std::fs::create_dir_all(mirror)?;
    let st = Command::new("git")
        .args(["init", "-q", "--bare"])
        .arg(mirror)
        .stdin(Stdio::null())
        .status()
        .context("git init")?;
    if !st.success() {
        bail!("git init --bare {} failed", mirror.display());
    }
    Ok(())
}

fn has_commit(mirror: &Path, sha: &str) -> bool {
    git(mirror, None)
        .args(["cat-file", "-e", &format!("{sha}^{{commit}}")])
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// The missing objects of `sha`, shallow: the tree we need, not the history.
fn fetch(mirror: &Path, url: &str, sha: &str, auth: &Auth) -> anyhow::Result<()> {
    let st = git(mirror, Some(auth))
        .args(["fetch", "-q", "--no-tags", "--depth=1", url, sha])
        .stderr(Stdio::inherit())
        .status()
        .context("git fetch")?;
    if !st.success() {
        bail!("git fetch {url} {sha} failed");
    }
    Ok(())
}

/// Checks the commit's tree out of the mirror into a scratch directory with
/// a scratch index (exact modes and symlinks, no export-ignore filtering),
/// imports it, and removes the scratch.
fn import_commit(
    cas: &Cas,
    mirror: &Path,
    sha: &str,
    url: &str,
    state_dir: &Path,
) -> anyhow::Result<Tree> {
    let scratch = state_dir
        .join("tmp")
        .join(format!("checkout-{}", ulid::Ulid::generate()));
    std::fs::create_dir_all(&scratch)?;
    let index = scratch.with_extension("index");
    let result = (|| -> anyhow::Result<Tree> {
        for args in [vec!["read-tree", sha], vec!["checkout-index", "-a", "-f"]] {
            let st = git(mirror, None)
                .arg(format!("--work-tree={}", scratch.display()))
                .env("GIT_INDEX_FILE", &index)
                .args(&args)
                .stderr(Stdio::inherit())
                .status()
                .with_context(|| format!("git {}", args[0]))?;
            if !st.success() {
                bail!("git {} {sha} failed", args[0]);
            }
        }
        git_dir(&scratch, mirror, sha, url)?;
        Ok(cas.import_dir(&scratch)?)
    })();
    let _ = sp_cas::remove_any(&scratch);
    let _ = std::fs::remove_file(&index);
    result
}

/// A `.git` for the checkout, as actions/checkout leaves one for a commit
/// (detached HEAD, `origin` the repository, depth 1), and nothing that
/// would make it differ from run to run or leak: no credentials (persisted
/// credentials would be a secret in the tree), no FETCH_HEAD (it names the
/// mirror's host path), no reflogs (times and identities), no hook samples.
/// It is part of the tree, so everything a step reads in it is observed:
/// go build's VCS stamping, `git describe`, `git status`.
fn git_dir(work: &Path, mirror: &Path, sha: &str, url: &str) -> anyhow::Result<()> {
    let run = |args: &[&str]| -> anyhow::Result<()> {
        let st = Command::new("git")
            .arg("-C")
            .arg(work)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_DATE", "1970-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "1970-01-01T00:00:00Z")
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .with_context(|| format!("git {}", args[0]))?;
        if !st.success() {
            bail!("git {args:?} failed in the checkout's .git");
        }
        Ok(())
    };
    run(&["init", "-q", "--template="])?;
    run(&["config", "core.logAllRefUpdates", "false"])?;
    run(&["config", "gc.auto", "0"])?;
    run(&["remote", "add", "origin", url])?;
    let mirror = mirror.to_string_lossy().into_owned();
    run(&[
        "fetch",
        "-q",
        "--no-tags",
        "--depth=1",
        "--no-write-fetch-head",
        &mirror,
        sha,
    ])?;
    run(&["update-ref", "--no-deref", "HEAD", sha])?;
    run(&["read-tree", sha])?;
    for leftover in ["FETCH_HEAD", "logs", "ORIG_HEAD"] {
        let p = work.join(".git").join(leftover);
        if p.exists() {
            sp_cas::remove_any(&p)?;
        }
    }
    Ok(())
}

pub fn tree_to_json(tree: &Tree) -> anyhow::Result<Vec<u8>> {
    let entries: Vec<&Entry> = tree.iter().collect();
    Ok(serde_json::to_vec(&entries)?)
}

pub fn tree_from_json(bytes: &[u8]) -> anyhow::Result<Tree> {
    let entries: Vec<Entry> = serde_json::from_slice(bytes)?;
    Ok(Tree::from_leaves(entries))
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sp_cas::{Hash, Kind};

    #[test]
    fn base64_matches_rfc() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"x-access-token:abc"), "eC1hY2Nlc3MtdG9rZW46YWJj");
    }

    #[test]
    fn tree_json_round_trips() {
        let t = Tree::from_leaves([
            Entry::file("a/b.go", Hash::of(b"b"), 0o644),
            Entry::symlink("l", "a/b.go"),
            Entry::dir("empty", Hash::of(b"")),
        ]);
        let back = tree_from_json(&tree_to_json(&t).unwrap()).unwrap();
        assert_eq!(back, t);
        assert!(back.get("empty").is_some_and(|e| e.k == Kind::Dir));
    }

    /// A local repository stands in for GitHub: ls-remote, shallow fetch of
    /// a commit by sha, exact tree import, and the memo on the second call.
    #[test]
    fn checkout_from_a_local_repo() {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("no git; skipping");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("pkg")).unwrap();
        std::fs::write(src.join("pkg/a.go"), "package pkg\n").unwrap();
        std::fs::write(src.join("README.md"), "# x\n").unwrap();
        let run = |args: &[&str]| {
            let st = Command::new("git")
                .arg("-C")
                .arg(&src)
                .args(args)
                .status()
                .unwrap();
            assert!(st.success(), "git {args:?}");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["add", "-A"]);
        run(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "one",
        ]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(src.join("run.sh"), "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(src.join("run.sh"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        run(&["add", "-A"]);
        run(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "-m",
            "two",
        ]);
        let head = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&src)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        let state = tmp.path().join("state");
        let cas = Cas::open(state.join("cas")).unwrap();
        let inputs = CheckoutInputs {
            repository: "o/r".into(),
            reference: "main".into(),
            token: None,
            server_url: format!("file://{}", tmp.path().display()),
            github_sha: String::new(),
            github_ref: String::new(),
            github_repository: "o/r".into(),
        };
        // The "server" is file://<tmp>/o/r: a symlink to the source repo.
        std::fs::create_dir_all(tmp.path().join("o")).unwrap();
        std::os::unix::fs::symlink(&src, tmp.path().join("o/r")).unwrap();
        // file:// shallow fetches need the local transport to allow them.
        let c = checkout(&state, &cas, &inputs).unwrap();
        assert_eq!(c.sha, head);
        assert!(!c.memo);
        assert_eq!(
            c.tree.get("pkg/a.go").unwrap().h,
            Some(Hash::of(b"package pkg\n"))
        );
        assert_eq!(c.tree.get("run.sh").unwrap().m, Some(0o755));
        // A .git as actions/checkout leaves one, without credentials or
        // anything that varies: detached HEAD at the commit, origin set.
        assert_eq!(
            cas.get(&c.tree.get(".git/HEAD").unwrap().h.unwrap())
                .unwrap()
                .unwrap(),
            format!("{head}\n").as_bytes()
        );
        assert!(c.tree.get(".git/FETCH_HEAD").is_none() && c.tree.get(".git/logs").is_none());
        let config = String::from_utf8(
            cas.get(&c.tree.get(".git/config").unwrap().h.unwrap())
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            config.contains("[remote \"origin\"]") && !config.contains("extraheader"),
            "{config}"
        );
        let again = checkout(&state, &cas, &inputs).unwrap();
        assert!(again.memo && again.tree == c.tree);
        assert_eq!(c.ref_output, "main");
        // By sha, without a ref: the ref output is empty.
        let by_sha = CheckoutInputs {
            reference: head.clone(),
            ..inputs.clone()
        };
        let s = checkout(&state, &cas, &by_sha).unwrap();
        assert!(s.memo && s.ref_output.is_empty());
        // Exact names only: a prefix of a branch is no branch, and a full
        // ref name works.
        let prefix = CheckoutInputs {
            reference: "mai".into(),
            ..inputs.clone()
        };
        assert!(checkout(&state, &cas, &prefix).is_err());
        let full = CheckoutInputs {
            reference: "refs/heads/main".into(),
            ..inputs.clone()
        };
        assert_eq!(checkout(&state, &cas, &full).unwrap().sha, head);
        // A repository the token cannot read gets nothing, not even the memo.
        let unreadable = CheckoutInputs {
            repository: "o/missing".into(),
            reference: head.clone(),
            ..inputs.clone()
        };
        assert!(checkout(&state, &cas, &unreadable).is_err());
    }
}
