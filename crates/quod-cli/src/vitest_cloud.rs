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
use quod_wire::testreplay::{LedgerUsed, TOP_DIRS, TOP_FILES, TestReport};
use quod_wire::testreplay_client::TestReplayClient;

pub struct Cloud {
    pub client: TestReplayClient,
    pub suite: String,
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

/// The ledger entries of `dir`.
fn entries(dir: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    TOP_FILES.iter().chain(TOP_DIRS).map(move |e| dir.join(e))
}

impl Cloud {
    /// Fetches the nearest ledger into `dir`, replacing the local one. When
    /// the server has none, or anything fails, the local one stays as it
    /// is (none on a fresh runner: every test file runs).
    pub fn restore(&self, root: &Path, dir: &Path) -> LedgerUsed {
        let t = Instant::now();
        let none = LedgerUsed::default();
        let anc = ancestors(root);
        let resp = match self.client.lookup(&self.suite, &anc) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "quod vitest: Quod Cloud lookup failed ({}); using the local ledger, if any",
                    e.0
                );
                return none;
            }
        };
        let Some(found) = resp.ledger else {
            eprintln!(
                "quod vitest: Quod Cloud has no ledger for {} yet ({} ms); using the local ledger, if any",
                self.suite,
                t.elapsed().as_millis()
            );
            return none;
        };
        let staging = dir.join(format!(".cloud-restore-{}", std::process::id()));
        remove(&staging);
        if let Err(e) = std::fs::create_dir_all(&staging) {
            eprintln!(
                "quod vitest: Quod Cloud restore failed ({e}); using the local ledger, if any"
            );
            return none;
        }
        let fetched = self.client.fetch(&found.files, &staging);
        let bytes = match fetched {
            Ok(b) => b,
            Err(e) => {
                remove(&staging);
                eprintln!(
                    "quod vitest: Quod Cloud restore failed ({}); using the local ledger, if any",
                    e.0
                );
                return none;
            }
        };
        // Swap: the old ledger's entries out, the fetched ones in. A
        // ledger must never be half one and half another (results are
        // named by test id, not content), so a failure here leaves none.
        let swapped = (|| -> std::io::Result<()> {
            for e in entries(dir) {
                remove(&e);
            }
            for (from, to) in entries(&staging).zip(entries(dir)) {
                if from.exists() {
                    std::fs::rename(&from, &to)?;
                }
            }
            Ok(())
        })();
        remove(&staging);
        if let Err(e) = swapped {
            for e in entries(dir) {
                remove(&e);
            }
            eprintln!(
                "quod vitest: Quod Cloud restore failed ({e}); no ledger: every test file runs"
            );
            return none;
        }
        eprintln!(
            "quod vitest: restored the ledger of {} ({}, {}) from Quod Cloud: {} files, {} KB in {} ms",
            found
                .sha
                .as_deref()
                .map_or("an unknown commit", |s| &s[..s.len().min(12)]),
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
        LedgerUsed {
            found: true,
            id: Some(found.id),
            sha: found.sha,
            git_ref: found.git_ref,
            scope: Some(found.scope),
            matched: Some(found.matched),
        }
    }

    /// Saves `dir` to Quod Cloud (only what the server lacks goes up).
    pub fn save(&self, dir: &Path) {
        if !dir.join("ledger.json").is_file() {
            eprintln!("quod vitest: no ledger to save to Quod Cloud");
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
