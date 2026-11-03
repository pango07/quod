//! `quod vitest --cloud`: the ledger directory from and to Quod Cloud
//! instead of actions/cache, and the run reports the dashboard shows
//! (quod-wire's `testreplay`, docs/test-replay-cloud.md).
//!
//! Nothing here can fail the job or make a wrong replay: a lookup is used
//! only when its signature verifies and every file matches its hash, and it
//! replaces the local ledger only once all of it is on disk; any error
//! leaves no ledger (every test file runs) or the local one untouched, and
//! a save or a report that fails is a line in the log.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use quod_wire::client::ServerConfig;
use quod_wire::testreplay::{LedgerUsed, Requester, TOP_DIRS, TOP_FILES, TestReport, valid_sha};
use quod_wire::testreplay_client::TestReplayClient;

pub struct Cloud {
    pub client: TestReplayClient,
    pub suite: String,
    /// QUOD_VITEST_CLOUD_FALLBACK=1: when Quod Cloud gives this job no
    /// ledger (unreachable, a failed download, none recorded), use the
    /// local one (e.g. an actions/cache copy) instead of none. Off by
    /// default: a local ledger is not signed for this job, so only a
    /// workflow that trusts its cache, as it did before Quod Cloud, turns
    /// this on.
    pub fallback_local: bool,
}

/// The cloud config, if `--cloud` was given: the URL from the flag, else
/// `QUOD_SERVER`. `None` with a log line when it cannot be used.
pub fn from_args(url: Option<&str>, suite: Option<&str>, root: &Path) -> Option<Cloud> {
    let url = match url.filter(|u| !u.is_empty()) {
        Some(u) => u.to_string(),
        None => match std::env::var("QUOD_SERVER").ok().filter(|v| !v.is_empty()) {
            Some(u) => u,
            None => {
                eprintln!(
                    "quod vitest: --cloud without a URL and QUOD_SERVER is not set; Quod Cloud is not used"
                );
                return None;
            }
        },
    };
    let get = |k: &str| {
        if k == "QUOD_SERVER" {
            Some(url.clone())
        } else {
            std::env::var(k).ok()
        }
    };
    let cfg = match ServerConfig::from_env(get)? {
        Ok(c) => c,
        Err(e) => {
            eprintln!("quod vitest: Quod Cloud is not used ({e})");
            return None;
        }
    };
    let suite = suite
        .map(str::to_string)
        .or_else(|| {
            std::env::var("QUOD_VITEST_SUITE")
                .ok()
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| default_suite(root));
    if !quod_wire::testreplay::valid_suite(&suite) {
        eprintln!("quod vitest: suite {suite:?} is not a valid name; Quod Cloud is not used");
        return None;
    }
    Some(Cloud {
        client: TestReplayClient::new(cfg),
        suite,
        fallback_local: std::env::var("QUOD_VITEST_CLOUD_FALLBACK").is_ok_and(|v| v == "1"),
    })
}

/// The vitest project's directory relative to the repository root (`.` at
/// the root).
fn default_suite(root: &Path) -> String {
    let p = Command::new("git")
        .args(["rev-parse", "--show-prefix"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .trim_end_matches('/')
                .to_string()
        })
        .unwrap_or_default();
    if p.is_empty() { ".".into() } else { p }
}

/// Commits to prefer, nearest first: what the checkout has of HEAD's
/// history, then a pull request's head and base commits.
fn ancestors(root: &Path) -> Vec<String> {
    let mut v: Vec<String> = Command::new("git")
        .args(["rev-list", "--max-count=200", "HEAD"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if v.is_empty()
        && let Ok(s) = std::env::var("GITHUB_SHA")
    {
        v.push(s);
    }
    if let Some(ev) = std::env::var_os("GITHUB_EVENT_PATH")
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    {
        for k in ["head", "base"] {
            if let Some(s) = ev["pull_request"][k]["sha"].as_str() {
                v.push(s.to_string());
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|s| quod_wire::testreplay::valid_sha(s) && seen.insert(s.clone()));
    v
}

fn remove(p: &Path) {
    let _ = std::fs::remove_dir_all(p);
    let _ = std::fs::remove_file(p);
}

/// The ledger entries of `dir`, `ledger.json` last: a swap that is cut
/// short leaves no `ledger.json`, so no ledger at all.
fn entries(dir: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    TOP_DIRS.iter().chain(TOP_FILES).map(move |e| dir.join(e))
}

/// Removes the local ledger (`ledger.json` first); an error if any of it
/// is still there.
fn clear(dir: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_file(dir.join(MARKER));
    let all: Vec<PathBuf> = entries(dir).collect();
    for e in all.iter().rev() {
        remove(e);
        if std::fs::symlink_metadata(e).is_ok() {
            return Err(std::io::Error::other(format!(
                "{} could not be removed",
                e.display()
            )));
        }
    }
    Ok(())
}

/// The restore marker in the ledger directory.
const MARKER: &str = ".quod-cloud-restore.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct Marker {
    tag: String,
    used: LedgerUsed,
}

/// This job's run, attempt and job, and the suite; `None` outside GitHub
/// Actions (no marker: every `--cloud` run restores).
fn job_tag(suite: &str) -> Option<String> {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    Some(format!(
        "{}/{}/{}/{suite}",
        get("GITHUB_RUN_ID")?,
        get("GITHUB_RUN_ATTEMPT")?,
        get("GITHUB_JOB")?
    ))
}

/// This job as GitHub describes it: a lookup answer must be for it.
fn requester() -> Requester {
    let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    Requester {
        repository: get("GITHUB_REPOSITORY"),
        repository_id: get("GITHUB_REPOSITORY_ID").and_then(|v| v.parse().ok()),
        git_ref: get("GITHUB_REF"),
    }
}

impl Cloud {
    /// Fetches the nearest ledger into `dir`. With --cloud the only ledger
    /// used is one Quod Cloud signed for this job: whatever was in `dir`
    /// before (a cache, files committed to the repository, an earlier job's
    /// on a persistent runner) is removed first, so when the server has
    /// none, or anything fails, there is no ledger and every test file
    /// runs.
    ///
    /// A restore that completed (a ledger, or the server's word that there
    /// is none) leaves a marker naming this job's run, attempt and job and
    /// the suite; a later `--cloud` run in the same job uses that ledger
    /// instead of fetching it again, so the restore can run in parallel with
    /// the dependency install (`--cloud-only restore &`). A marker from any
    /// other job (a cache, a committed file, a persistent runner) does not
    /// match and is removed with the rest.
    pub fn restore(&self, root: &Path, dir: &Path) -> LedgerUsed {
        let marker = dir.join(MARKER);
        let here = job_tag(&self.suite);
        if let Some(tag) = &here
            && let Some(m) = std::fs::read(&marker)
                .ok()
                .and_then(|b| serde_json::from_slice::<Marker>(&b).ok())
            && m.tag == *tag
            && (!m.used.found || dir.join("ledger.json").is_file())
        {
            eprintln!("quod vitest: using the ledger restored from Quod Cloud earlier in this job");
            return m.used;
        }
        let _ = std::fs::remove_file(&marker);
        let (used, definitive) = self.restore_fresh(root, dir);
        if definitive && let Some(tag) = here {
            let m = Marker {
                tag,
                used: used.clone(),
            };
            if let Ok(b) = serde_json::to_vec(&m) {
                let _ = std::fs::write(&marker, b);
            }
        }
        used
    }

    /// The restore itself: (what was used, whether it completed: a ledger
    /// or the server's word that it has none).
    fn restore_fresh(&self, root: &Path, dir: &Path) -> (LedgerUsed, bool) {
        let t = Instant::now();
        let none = (LedgerUsed::default(), false);
        // With the fallback the local ledger stays until a fetched one has
        // verified (the swap below replaces it); without it, it goes now.
        let local = self.fallback_local && dir.join("ledger.json").is_file();
        if local {
            eprintln!(
                "quod vitest: QUOD_VITEST_CLOUD_FALLBACK=1: the local ledger is used if Quod Cloud gives none"
            );
        } else if let Err(e) = clear(dir) {
            eprintln!(
                "quod vitest: the local ledger could not be removed ({e}); Quod Cloud is not used"
            );
            return none;
        }
        let anc = ancestors(root);
        let resp = match self.client.lookup(&self.suite, &anc, &requester()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "quod vitest: Quod Cloud lookup failed ({}); no ledger: every test file runs",
                    e.0
                );
                return none;
            }
        };
        let Some(found) = resp.ledger else {
            eprintln!(
                "quod vitest: Quod Cloud has no ledger for {} yet ({} ms); every test file runs",
                self.suite,
                t.elapsed().as_millis()
            );
            return (LedgerUsed::default(), true);
        };
        let staging = dir.join(format!(".cloud-restore-{}", std::process::id()));
        remove(&staging);
        if let Err(e) = std::fs::create_dir_all(&staging) {
            eprintln!("quod vitest: Quod Cloud restore failed ({e}); every test file runs");
            return none;
        }
        let fetched = self
            .client
            .fetch(&found.files, &staging, found.bundle.as_ref());
        let (bytes, how) = match fetched {
            Ok(b) => b,
            Err(e) => {
                remove(&staging);
                eprintln!(
                    "quod vitest: Quod Cloud restore failed ({}); every test file runs",
                    e.0
                );
                return none;
            }
        };
        // Move the fetched entries in, `ledger.json` last. A ledger must
        // never be half one and half another (results are named by test
        // id, not content); `dir` holds none now, and a failure leaves
        // none.
        let swapped = (|| -> std::io::Result<()> {
            for (from, to) in entries(&staging).zip(entries(dir)) {
                if from.exists() {
                    std::fs::rename(&from, &to)?;
                }
            }
            Ok(())
        })();
        remove(&staging);
        if let Err(e) = swapped {
            let _ = clear(dir);
            eprintln!(
                "quod vitest: Quod Cloud restore failed ({e}); no ledger: every test file runs"
            );
            return none;
        }
        eprintln!(
            "quod vitest: restored the ledger of {} ({}, {}) from Quod Cloud: {} files, {} KB ({how}) in {} ms",
            found
                .sha
                .as_deref()
                .filter(|s| valid_sha(s))
                .map_or("an unknown commit", |s| &s[..12]),
            found.git_ref.as_deref().unwrap_or(&found.scope),
            match found.matched.as_str() {
                "ancestor" => "nearest recorded commit",
                "branch" => "newest of this branch",
                _ => "newest of the default branch",
            },
            found.files.len(),
            bytes / 1024,
            t.elapsed().as_millis()
        );
        (
            LedgerUsed {
                found: true,
                id: Some(found.id),
                sha: found.sha,
                git_ref: found.git_ref,
                scope: Some(found.scope),
                matched: Some(found.matched),
            },
            true,
        )
    }

    /// Saves `dir` to Quod Cloud (only what the server lacks goes up).
    pub fn save(&self, dir: &Path) {
        if !dir.join("ledger.json").is_file() {
            eprintln!("quod vitest: no ledger to save to Quod Cloud");
            return;
        }
        if let Some(k) = crate::vitest::credential_in(dir) {
            eprintln!(
                "quod vitest: the ledger was not saved to Quod Cloud: it contains the value of {k}"
            );
            return;
        }
        let t = Instant::now();
        match self.client.save(&self.suite, dir) {
            Ok((r, n, bytes)) => {
                eprintln!(
                    "quod vitest: saved the ledger to Quod Cloud (scope {}, {} new files, {} KB uploaded, {} ms)",
                    r.scope,
                    n,
                    bytes / 1024,
                    t.elapsed().as_millis()
                );
                if let Some(n) = r.notice {
                    eprintln!("quod: {n}");
                }
            }
            Err(e) => eprintln!(
                "quod vitest: the ledger was not saved to Quod Cloud ({})",
                e.0
            ),
        }
    }

    pub fn report(&self, r: &TestReport) {
        if let Err(e) = self.client.report(&self.suite, r) {
            eprintln!(
                "quod vitest: the run report was not sent to Quod Cloud ({})",
                e.0
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quod_wire::client::Auth;

    /// A server that answers every request 502 (a deploy in progress).
    fn bad_gateway() -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let a = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut s in l.incoming().flatten() {
                let mut b = [0u8; 8192];
                let _ = std::io::Read::read(&mut s, &mut b);
                let _ = std::io::Write::write_all(
                    &mut s,
                    b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                );
            }
        });
        format!("http://{a}")
    }

    fn cloud(url: &str, fallback_local: bool) -> Cloud {
        Cloud {
            client: TestReplayClient::new(ServerConfig {
                url: url.into(),
                auth: Auth::Token("t".into()),
                server_key: None,
                timeout: std::time::Duration::from_secs(5),
                scope: None,
                ci: None,
            }),
            suite: ".".into(),
            fallback_local,
        }
    }

    #[test]
    fn a_502_keeps_the_local_ledger_only_with_the_fallback() {
        let url = bad_gateway();
        for fallback in [true, false] {
            let t = tempfile::tempdir().unwrap();
            std::fs::write(t.path().join("ledger.json"), "{}").unwrap();
            std::fs::create_dir_all(t.path().join("results")).unwrap();
            std::fs::write(t.path().join("results/a.json"), "a").unwrap();
            let used = cloud(&url, fallback).restore(t.path(), t.path());
            assert!(!used.found, "{fallback}");
            // With the fallback the local ledger is used whole; without
            // it, no ledger (every file runs).
            assert_eq!(t.path().join("ledger.json").is_file(), fallback);
            assert_eq!(t.path().join("results/a.json").is_file(), fallback);
            // A failed lookup leaves no marker: the next --cloud run tries again.
            assert!(!t.path().join(MARKER).exists());
        }
    }
}
