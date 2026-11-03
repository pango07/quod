//! The dashboard's read API (cloud/docs/api.md, `CloudApi` in
//! cloud/lib/api/types.ts), from what the server has: the team's step rows
//! (`store::Activity`) and its scope ledgers. Trust state is the trusted
//! scope's (what default-branch runs replay), or, for a step the default
//! branch has not recorded, the newest other scope's (`trustRef` names that
//! branch); every scope's rows count in runs and totals. Endpoints whose
//! data the server does not have yet (team resolution from WorkOS, re-runs,
//! the proofs toggle, runners, the Connect choice) answer 501 with a
//! message saying so.
//!
//! Viewers authenticate with their WorkOS access token (the path's team is
//! then the team's WorkOS organization id) or a static viewer token
//! (`auth::Auth::viewer`). A team that is not the viewer's is 404, never
//! 403, so existence does not leak.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post, put};
use futures_util::Stream;
use quod_cas::{Hash, Kind};
use quod_ledger::{TrustRow, TrustState, now_rfc3339};
use quod_trust::{TRAILING_AUDITS, Verdict};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{Role, TRUSTED, Viewer};
use crate::store::{EventRow, StepRow, TrustedWrite};
use crate::{ApiError, App, AppState, blocking, valid_id};

type Json = axum::Json<Value>;

/// GitHub's Linux runner price, for "est. cost saved".
const USD_PER_MINUTE: f64 = 0.008;
const RESERVED: &[&str] = &[
    "repositories",
    "runs",
    "trust",
    "runners",
    "usage",
    "settings",
    "connect",
];

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/teams/resolve", post(resolve))
        .route("/v1/teams/{team}/health", get(health))
        .route("/v1/teams/{team}/overview", get(overview))
        .route("/v1/teams/{team}/repos", get(repos))
        .route("/v1/teams/{team}/repos/{repo}", get(repo))
        .route("/v1/teams/{team}/runs", get(runs))
        .route("/v1/teams/{team}/repos/{repo}/runs/{number}", get(run))
        .route(
            "/v1/teams/{team}/repos/{repo}/runs/{number}/rerun",
            post(rerun),
        )
        .route("/v1/teams/{team}/steps", get(steps))
        .route("/v1/teams/{team}/repos/{repo}/steps/{key}", get(step))
        .route(
            "/v1/teams/{team}/repos/{repo}/steps/{key}/proofs",
            put(set_proofs),
        )
        .route("/v1/teams/{team}/runners", get(runners))
        .route("/v1/teams/{team}/connect", get(connect).put(save_connect))
        .route("/v1/teams/{team}/search-index", get(search_index))
        .route("/v1/teams/{team}/activity", get(activity))
        .route("/v1/teams/{team}/events", get(events))
        .route(
            "/v1/teams/{team}/test-runs",
            get(crate::testreplay::list_runs),
        )
        .route(
            "/v1/teams/{team}/test-runs/{id}",
            get(crate::testreplay::get_run),
        )
        .route("/v1/teams/{team}/test-stats", get(crate::testreplay::stats))
}

/// The viewer, for the team the path names (its slug or WorkOS org id):
/// 404 for any team that is not theirs.
pub(crate) fn viewer(app: &App, headers: &HeaderMap, team: &str) -> Result<Viewer, ApiError> {
    if !valid_id(team) {
        return Err(ApiError::not_found("no such team"));
    }
    app.auth.viewer(headers, team)
}

/// An admin of the team, for the endpoints that change settings.
fn admin(app: &App, headers: &HeaderMap, team: &str) -> Result<Viewer, ApiError> {
    let v = viewer(app, headers, team)?;
    if v.role != Role::Admin {
        return Err(ApiError::forbidden("only a team admin can change this"));
    }
    Ok(v)
}

// ----- the team's data, gathered once per request -----

/// A repository's dashboard slug: its name, or `owner-name` when two
/// owners share the name; a reserved page name gets the owner too.
pub fn slug(repository: &str, all: &BTreeSet<String>) -> String {
    let clean = |s: &str| -> String {
        s.to_ascii_lowercase()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    };
    let (owner, name) = repository.split_once('/').unwrap_or(("", repository));
    let shared = all
        .iter()
        .filter(|r| r.split_once('/').map(|x| x.1) == Some(name))
        .count()
        > 1;
    if shared || RESERVED.contains(&clean(name).as_str()) {
        format!("{}-{}", clean(owner), clean(name))
    } else {
        clean(name)
    }
}

struct TeamData {
    rows: Vec<StepRow>,
    repos: BTreeSet<String>,
    /// Trust rows by (repository, key hex): the trusted scope's, else (a key
    /// only branch or pull request runs have recorded) the most recently
    /// updated other scope's. Reading the trusted scope alone made such a
    /// step "proving 0 of N" while its own runs said "proof 1 of 1".
    trust: BTreeMap<(String, String), TrustRow>,
    /// The ref whose scope a `trust` row came from, for a row that is not
    /// the trusted scope's (`trustRef` on StepSummary).
    trust_ref: BTreeMap<(String, String), String>,
    /// Best proof streak per (repository, key hex), in the row's scope.
    streaks: BTreeMap<(String, String), u32>,
    /// Audits of the trusted scopes: (repository, key hex, audit). Team
    /// totals (spot checks, wrong replays) count these only.
    audits: Vec<(String, String, quod_ledger::AuditRow)>,
    /// Audits of the keys whose `trust` row is another scope's, from that
    /// scope (their step page's history and divergence).
    branch_audits: Vec<(String, String, quod_ledger::AuditRow)>,
    recent_div: BTreeMap<(String, String), u64>,
    proofs: u32,
    /// Writes into each repository's trusted scope, by ref.
    writes: Vec<TrustedWrite>,
    team: Option<crate::config::TeamConfig>,
    /// The team's id on this server (the path may have named its org id).
    id: String,
}

impl TeamData {
    fn load(app: &App, team: &str) -> Result<TeamData, ApiError> {
        let act = app.store.activity(team)?;
        let rows = act.steps(None)?;
        let writes = act.trusted_writes(None)?;
        let mut repos: BTreeSet<String> = rows.iter().map(|r| r.repository.clone()).collect();
        let mut trust: BTreeMap<(String, String), TrustRow> = BTreeMap::new();
        let mut trust_ref = BTreeMap::new();
        let mut streaks = BTreeMap::new();
        let mut audits = vec![];
        let mut branch_audits = vec![];
        let mut recent_div = BTreeMap::new();
        // Pass 1: each key's row, from the scope that decides it.
        let scopes = app.store.scopes(team);
        let mut ledgers = Vec::with_capacity(scopes.len());
        // (repository, key) -> (trusted, updated_at, index into `ledgers`)
        let mut owner: BTreeMap<(String, String), (bool, String, usize)> = BTreeMap::new();
        for (i, s) in scopes.iter().enumerate() {
            repos.insert(s.repository.clone());
            let trusted = s.scope == TRUSTED;
            let l = crate::store::open_ledger(&s.path)?;
            for t in l.trust_rows()? {
                let k = (s.repository.clone(), t.step_key.hex());
                let better = match owner.get(&k) {
                    None => true,
                    Some((o_trusted, o_at, _)) => {
                        (trusted && !o_trusted) || (trusted == *o_trusted && t.updated_at > *o_at)
                    }
                };
                if better {
                    owner.insert(k.clone(), (trusted, t.updated_at.clone(), i));
                    trust.insert(k, t);
                }
            }
            ledgers.push(l);
        }
        // Pass 2: streaks, recent divergences and audits, from that scope.
        for (i, s) in scopes.iter().enumerate() {
            let trusted = s.scope == TRUSTED;
            let l = &ledgers[i];
            let mine: BTreeSet<(String, String)> = owner
                .iter()
                .filter(|(_, (_, _, at))| *at == i)
                .map(|(k, _)| k.clone())
                .collect();
            for k in &mine {
                let key = trust[k].step_key;
                let best = l
                    .proofs_for_key(&key)?
                    .iter()
                    .map(|p| p.streak)
                    .max()
                    .unwrap_or(0);
                streaks.insert(k.clone(), best);
                recent_div.insert(k.clone(), l.recent_divergences(&key, TRAILING_AUDITS)?);
                if !trusted {
                    // The ref of that scope's newest row of the key.
                    let r = rows
                        .iter()
                        .rev()
                        .find(|r| r.repository == k.0 && r.scope == s.scope && key_hex(r) == k.1)
                        .map(|r| branch(r.git_ref.as_deref()))
                        .unwrap_or_default();
                    trust_ref.insert(k.clone(), r);
                }
            }
            if trusted {
                for a in l.audits(None, 10_000)? {
                    audits.push((s.repository.clone(), a.step_key.hex(), a));
                }
            } else if !mine.is_empty() {
                for a in l.audits(None, 10_000)? {
                    let k = (s.repository.clone(), a.step_key.hex());
                    if mine.contains(&k) {
                        branch_audits.push((k.0, k.1, a));
                    }
                }
            }
        }
        let proofs = app
            .auth
            .team(team)
            .map_or(quod_trust::DEFAULT_PROOFS, |t| t.policy.proofs);
        Ok(TeamData {
            rows,
            repos,
            trust,
            trust_ref,
            streaks,
            audits,
            branch_audits,
            recent_div,
            proofs,
            writes,
            team: app.auth.team(team).cloned(),
            id: team.to_string(),
        })
    }

    fn require_protected_ref(&self, repo: Option<&str>) -> bool {
        match (&self.team, repo) {
            (Some(t), Some(r)) => t.requires_protected_ref(r),
            (Some(t), None) => t.require_protected_ref,
            (None, _) => false,
        }
    }

    /// Where a repository's trusted results come from, and the warning
    /// when a ref that writes them is not known to be protected and the
    /// repository does not require it (DECISIONS 2026-11-02).
    fn writer_trust(&self, repo: &str) -> (Value, Value) {
        let ws: Vec<&TrustedWrite> = self
            .writes
            .iter()
            .filter(|w| w.repository == repo)
            .collect();
        let list: Vec<Value> = ws
            .iter()
            .map(|w| json!({"ref": w.git_ref, "protected": w.protected, "lastAt": w.last_at, "writes": w.writes}))
            .collect();
        let warning = if self.require_protected_ref(Some(repo)) {
            None
        } else {
            ws.iter()
                .filter(|w| w.protected != Some(true))
                .max_by(|a, b| a.last_at.cmp(&b.last_at))
                .map(|w| crate::api::writer_warning(repo, &w.git_ref, w.protected))
        };
        (Value::Array(list), json!(warning))
    }

    /// Only what one step row's event needs: the team's repository names
    /// (for the slug), that key's trust in its repository's trusted scope,
    /// and the run number of the run a replay came from. Not the team's
    /// every row, which `load` reads (once per step otherwise).
    fn for_row(app: &App, team: &str, row: &StepRow) -> Result<TeamData, ApiError> {
        let scopes = app.store.scopes(team);
        let mut repos: BTreeSet<String> = scopes.iter().map(|s| s.repository.clone()).collect();
        repos.insert(row.repository.clone());
        let (mut trust, mut streaks, mut recent_div) =
            (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        let key = row.step_key.as_deref().and_then(|k| k.parse::<Hash>().ok());
        // The trusted scope's row of the key, else the row's own scope's
        // (as `load` falls back).
        for scope in [TRUSTED, row.scope.as_str()] {
            let Some(key) = key else { break };
            let Some(s) = scopes
                .iter()
                .find(|s| s.repository == row.repository && s.scope == scope)
            else {
                continue;
            };
            let l = crate::store::open_ledger(&s.path)?;
            if let Some(t) = l.trust(&key)? {
                let k = (row.repository.clone(), key.hex());
                let best = l.proofs_for_key(&key)?.iter().map(|p| p.streak).max();
                streaks.insert(k.clone(), best.unwrap_or(0));
                recent_div.insert(k.clone(), l.recent_divergences(&key, TRAILING_AUDITS)?);
                trust.insert(k, t);
                break;
            }
        }
        let mut rows = vec![];
        if let Some(src) = &row.source_run
            && let Some(n) = app.store.activity(team)?.run_number_of(src)?
        {
            rows.push(StepRow {
                quod_run: src.clone(),
                run_number: Some(n),
                ..StepRow::default()
            });
        }
        let cfg = app.auth.team(team).cloned();
        Ok(TeamData {
            rows,
            repos,
            trust,
            trust_ref: BTreeMap::new(),
            streaks,
            audits: vec![],
            branch_audits: vec![],
            recent_div,
            proofs: cfg
                .as_ref()
                .map_or(quod_trust::DEFAULT_PROOFS, |t| t.policy.proofs),
            writes: vec![],
            team: cfg,
            id: team.to_string(),
        })
    }

    fn slug(&self, repository: &str) -> String {
        slug(repository, &self.repos)
    }

    fn repo_by_slug(&self, s: &str) -> Option<String> {
        self.repos.iter().find(|r| self.slug(r) == s).cloned()
    }

    fn needed(&self, repo: &str, key: &str) -> u32 {
        let d = self
            .recent_div
            .get(&(repo.to_string(), key.to_string()))
            .copied()
            .unwrap_or(0);
        self.proofs + (d.min(u64::from(quod_trust::MAX_EXTRA_PROOFS)) as u32)
    }

    /// The dashboard's TrustState of a key.
    fn trust_state(&self, repo: &str, key: &str) -> Value {
        let k = (repo.to_string(), key.to_string());
        let need = self.needed(repo, key);
        let have = self.streaks.get(&k).copied().unwrap_or(0);
        match self.trust.get(&k) {
            Some(t) if t.state == TrustState::Quarantined => {
                json!({"kind": "paused", "consistent": have, "need": need})
            }
            Some(t) if t.state == TrustState::Proven && t.replays == 0 && t.eligible > 0 => {
                json!({"kind": "watching", "wouldHaveReplayed": t.eligible})
            }
            Some(t) if t.state == TrustState::Proven => json!({"kind": "replaying"}),
            _ => json!({"kind": "proving", "have": have.min(need), "need": need}),
        }
    }

    fn source_run_number(&self, quod_run: &str) -> u64 {
        self.rows
            .iter()
            .find(|r| r.quod_run == quod_run)
            .and_then(|r| r.run_number)
            .unwrap_or(0)
    }

    fn divergences(&self, repo: &str, key: &str) -> u64 {
        self.trust
            .get(&(repo.to_string(), key.to_string()))
            .map_or(0, |t| t.divergences)
    }
}

fn key_hex(r: &StepRow) -> String {
    r.step_key
        .as_deref()
        .and_then(|k| k.parse::<Hash>().ok())
        .map(|h| h.hex())
        .unwrap_or_default()
}

fn secs(ms: Option<u64>) -> f64 {
    ms.unwrap_or(0) as f64 / 1000.0
}

fn minutes(ms: u64) -> u64 {
    (ms as f64 / 60_000.0).round() as u64
}

fn replayed(r: &StepRow) -> bool {
    r.outcome == "replayed"
}

/// The `why` of a step row (StepWhy).
fn why(d: &TeamData, r: &StepRow) -> Value {
    let key = key_hex(r);
    let proof = json!({"have": r.streak.unwrap_or(0), "need": r.needed.unwrap_or(d.proofs)});
    match r.outcome.as_str() {
        "replayed" => json!({
            "kind": "replayed", "stepKey": key,
            "sourceRun": r.source_run.as_deref().map_or(0, |s| d.source_run_number(s)),
            "inputsMatched": r.inputs.unwrap_or(0), "proof": proof,
            "divergences": d.divergences(&r.repository, &key),
            "savedSeconds": secs(r.saved_ms),
        }),
        "audit" => {
            json!({"kind": "spot-check", "stepKey": key, "sourceRun": 0, "inputsMatched": r.inputs.unwrap_or(0), "identical": true})
        }
        "diverged" => {
            json!({"kind": "spot-check", "stepKey": key, "sourceRun": 0, "inputsMatched": r.inputs.unwrap_or(0), "identical": false})
        }
        "would-replay" => {
            json!({"kind": "watching", "stepKey": key, "couldHaveReplayed": true, "recordedInputs": r.inputs.unwrap_or(0)})
        }
        "unprovable" => {
            json!({"kind": "watching", "stepKey": key, "couldHaveReplayed": false, "recordedInputs": r.inputs.unwrap_or(0)})
        }
        "tainted" | "net" => json!({"kind": "network"}),
        "proving"
            if d.trust
                .get(&(r.repository.clone(), key.clone()))
                .is_some_and(|t| t.state == TrustState::Quarantined) =>
        {
            json!({"kind": "paused", "stepKey": key, "consistent": r.streak.unwrap_or(0), "need": r.needed.unwrap_or(d.proofs)})
        }
        _ => {
            json!({"kind": "first-run", "stepKey": key, "recordedInputs": r.inputs.unwrap_or(0), "proof": proof})
        }
    }
}

fn run_step(d: &TeamData, r: &StepRow) -> Value {
    let mut v = json!({
        "name": r.step_name.clone().or_else(|| r.cmd.clone()).unwrap_or_default(),
        "outcome": if replayed(r) { "replayed" } else { "ran" },
        "seconds": secs(r.wall_ms),
        "why": why(d, r),
    });
    // GitHub's 1-based step number, when the runner reported it.
    if let Some(i) = r.step_index {
        v["index"] = json!(i);
    }
    v
}

/// A step row's job: GitHub's job id when the runner sent it (matrix legs
/// apart), else the workflow's job id.
fn job_id(r: &StepRow) -> String {
    r.job_id
        .clone()
        .or_else(|| r.job.clone())
        .unwrap_or_else(|| "job".into())
}

/// A step key as the events carry it: hex, or null for a step without one.
fn key_or_null(r: &StepRow) -> Value {
    let k = key_hex(r);
    if k.is_empty() { Value::Null } else { json!(k) }
}

fn branch(git_ref: Option<&str>) -> String {
    git_ref
        .map(|r| r.strip_prefix("refs/heads/").unwrap_or(r).to_string())
        .unwrap_or_default()
}

/// One CI run attempt's step rows.
struct RunGroup<'a> {
    repo: String,
    number: u64,
    run_id: Option<String>,
    attempt: Option<u64>,
    rows: Vec<&'a StepRow>,
}

/// Rows grouped by run: GitHub's run id and attempt when the runner sent
/// them (two workflows share run numbers; attempts are separate runs), else
/// (repository, run number). Rows without a run number are left out (the
/// dashboard's paths name runs by number). Oldest first.
fn runs_of<'a>(d: &'a TeamData, repo: Option<&str>) -> Vec<RunGroup<'a>> {
    type Key = (String, String, u64);
    let mut m: BTreeMap<Key, Vec<&StepRow>> = BTreeMap::new();
    for r in &d.rows {
        if repo.is_some_and(|x| x != r.repository) {
            continue;
        }
        let Some(n) = r.run_number else { continue };
        let id = r.run_id.clone().unwrap_or_else(|| format!("#{n}"));
        m.entry((r.repository.clone(), id, r.run_attempt.unwrap_or(0)))
            .or_default()
            .push(r);
    }
    let mut v: Vec<RunGroup> = m
        .into_iter()
        .map(|((repo, _, _), rows)| RunGroup {
            repo,
            number: rows[0].run_number.unwrap_or(0),
            run_id: rows[0].run_id.clone(),
            attempt: rows[0].run_attempt,
            rows,
        })
        .collect();
    v.sort_by(|a, b| (&a.rows[0].at, a.number).cmp(&(&b.rows[0].at, b.number)));
    v
}

fn run_summary(d: &TeamData, g: &RunGroup) -> Value {
    let (repo, number, rows) = (g.repo.as_str(), g.number, g.rows.as_slice());
    let first = rows[0];
    let failed = rows.iter().any(|r| r.exit_code.is_some_and(|c| c != 0));
    let took: f64 = rows.iter().map(|r| secs(r.wall_ms)).sum();
    let would: f64 = rows
        .iter()
        .map(|r| {
            if replayed(r) {
                secs(r.recorded_ms)
            } else {
                secs(r.wall_ms)
            }
        })
        .sum();
    let event = first
        .event
        .as_deref()
        .filter(|e| {
            matches!(
                *e,
                "push" | "pull_request" | "schedule" | "workflow_dispatch"
            )
        })
        .unwrap_or("push");
    let spot = rows
        .iter()
        .filter(|r| matches!(r.outcome.as_str(), "audit" | "diverged"))
        .count();
    let identical = rows.iter().filter(|r| r.outcome == "audit").count();
    json!({
        "repoSlug": d.slug(repo),
        "number": number,
        "runId": g.run_id.as_deref().and_then(|s| s.parse::<u64>().ok()),
        "runAttempt": g.attempt,
        "title": first.workflow.clone().unwrap_or_else(|| format!("Run {number}")),
        "branch": branch(first.git_ref.as_deref()),
        "sha": first.sha.clone().unwrap_or_default(),
        "actor": first.actor.clone().unwrap_or_default(),
        "event": event,
        "runner": first.runner.clone().unwrap_or_default(),
        "status": if failed { "failed" } else { "passed" },
        "startedAt": first.at,
        "tookSeconds": took,
        "wouldHaveTakenSeconds": would,
        "stepsReplayed": rows.iter().filter(|r| replayed(r)).count(),
        "stepsTotal": rows.len(),
        "spotChecks": spot,
        "spotChecksIdentical": identical,
    })
}

fn run_detail(d: &TeamData, g: &RunGroup) -> Value {
    let mut v = run_summary(d, g);
    let repo = &g.repo;
    let mut jobs: BTreeMap<String, (String, Vec<Value>)> = BTreeMap::new();
    let mut order: Vec<String> = vec![];
    for r in &g.rows {
        let j = job_id(r);
        if !order.contains(&j) {
            order.push(j.clone());
        }
        let name = r
            .job_name
            .clone()
            .or_else(|| r.job.clone())
            .unwrap_or_else(|| j.clone());
        jobs.entry(j)
            .or_insert_with(|| (name, vec![]))
            .1
            .push(run_step(d, r));
    }
    v["githubUrl"] = json!(match (g.run_id.as_deref(), g.attempt) {
        (Some(id), Some(a)) => format!("https://github.com/{repo}/actions/runs/{id}/attempts/{a}"),
        (Some(id), None) => format!("https://github.com/{repo}/actions/runs/{id}"),
        (None, _) => format!("https://github.com/{repo}/actions"),
    });
    v["jobs"] = Value::Array(
        order
            .into_iter()
            .map(|j| {
                let (name, steps) = jobs.remove(&j).unwrap_or_default();
                json!({"id": j, "name": name, "steps": steps})
            })
            .collect(),
    );
    v
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Some(v[v.len() / 2])
}

/// StepSummary per (repository, key), for one repository or all.
fn step_summaries(d: &TeamData, repo: Option<&str>) -> Vec<Value> {
    let mut by: BTreeMap<(String, String), Vec<&StepRow>> = BTreeMap::new();
    for r in &d.rows {
        if repo.is_some_and(|x| x != r.repository) || r.step_key.is_none() {
            continue;
        }
        by.entry((r.repository.clone(), key_hex(r)))
            .or_default()
            .push(r);
    }
    by.into_iter()
        .map(|((repo, key), rows)| step_summary(d, &repo, &key, &rows))
        .collect()
}

fn step_summary(d: &TeamData, repo: &str, key: &str, rows: &[&StepRow]) -> Value {
    let trust = d.trust_state(repo, key);
    let n_replayed = rows.iter().filter(|r| replayed(r)).count();
    let replaying = trust["kind"] == "replaying";
    let last = rows.last().unwrap();
    json!({
        "repoSlug": d.slug(repo),
        "key": key,
        "job": last.job.clone().unwrap_or_default(),
        "command": last.cmd.clone().unwrap_or_default(),
        "trust": trust,
        "replayRate": if replaying { json!(n_replayed as f64 / rows.len() as f64) } else { Value::Null },
        "ranSeconds": median(rows.iter().filter(|r| !replayed(r)).map(|r| secs(r.wall_ms)).collect()).unwrap_or(0.0),
        "replaySeconds": median(rows.iter().filter(|r| replayed(r)).map(|r| secs(r.wall_ms)).collect()),
        "savedMinutes": minutes(rows.iter().filter_map(|r| r.saved_ms).sum()),
        // The branch whose own results `trust` describes, when the default
        // branch has not recorded the step (null: the default branch's).
        "trustRef": d.trust_ref.get(&(repo.to_string(), key.to_string())),
    })
}

fn repo_summary(d: &TeamData, repo: &str) -> Value {
    let rows: Vec<&StepRow> = d.rows.iter().filter(|r| r.repository == repo).collect();
    let runs = runs_of(d, Some(repo));
    let last = runs.last();
    let (writes, warning) = d.writer_trust(repo);
    let paused = d
        .trust
        .iter()
        .filter(|((r, _), t)| r == repo && t.state == TrustState::Quarantined)
        .count();
    json!({
        "slug": d.slug(repo),
        "fullName": repo,
        "language": "",
        "runner": "github-hosted",
        "replayRate": if rows.is_empty() { 0.0 } else { rows.iter().filter(|r| replayed(r)).count() as f64 / rows.len() as f64 },
        "savedMinutes": minutes(rows.iter().filter_map(|r| r.saved_ms).sum()),
        "lastRunAt": last.map(|g| json!(g.rows[0].at)).unwrap_or(Value::Null),
        "lastRunStatus": last.map(|g| run_summary(d, g)["status"].clone()).unwrap_or(Value::Null),
        "pausedSteps": paused,
        "hasQuodData": !rows.is_empty(),
        "trustedWrites": writes,
        "writerTrustWarning": warning,
    })
}

fn paused_refs(d: &TeamData) -> Vec<Value> {
    d.trust
        .iter()
        .filter(|(_, t)| t.state == TrustState::Quarantined)
        .map(|((repo, key), t)| {
            let job = d
                .rows
                .iter()
                .rev()
                .find(|r| r.repository == *repo && key_hex(r) == *key)
                .and_then(|r| r.job.clone())
                .unwrap_or_default();
            json!({"repoSlug": d.slug(repo), "repoName": repo, "key": key, "job": job, "command": t.cmd.clone().unwrap_or_default()})
        })
        .collect()
}

/// (spot checks, identical, wrong replays) among the trusted scopes' audits
/// of replay-mode hits.
fn spot_checks(d: &TeamData, since: Option<&str>) -> (usize, usize, usize) {
    let a: Vec<_> = d
        .audits
        .iter()
        .filter(|(_, _, a)| a.phase == "audit" && since.is_none_or(|s| a.at.as_str() >= s))
        .collect();
    let wrong = a.iter().filter(|(_, _, a)| a.diverged).count();
    (a.len(), a.len() - wrong, wrong)
}

// ----- handlers -----

async fn with_team<F>(
    app: AppState,
    headers: HeaderMap,
    team: String,
    f: F,
) -> Result<Json, ApiError>
where
    F: FnOnce(&App, &TeamData) -> Result<Value, ApiError> + Send + 'static,
{
    blocking(move || {
        let v = viewer(&app, &headers, &team)?;
        let d = TeamData::load(&app, &v.team)?;
        Ok(axum::Json(f(&app, &d)?))
    })
    .await
}

async fn resolve() -> ApiError {
    ApiError::not_implemented(
        "team resolution from WorkOS organizations is not built yet (P5); teams are configured on the server",
    )
}

async fn health(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(team): Path<String>,
) -> Result<Json, ApiError> {
    with_team(app, headers, team, |_, d| {
        let (spot, _, wrong) = spot_checks(d, None);
        Ok(json!({
            "wrongReplays": wrong,
            "spotChecks": spot,
            "pausedSteps": d.trust.values().filter(|t| t.state == TrustState::Quarantined).count(),
            "repos": d.repos.len(),
            "requireProtectedRef": d.require_protected_ref(None),
        }))
    })
    .await
}

#[derive(Deserialize)]
struct RangeQ {
    range: Option<String>,
}

fn since(range: &str) -> Result<String, ApiError> {
    let days = match range {
        "7d" => 7,
        "30d" => 30,
        "90d" => 90,
        _ => return Err(ApiError::bad_request("range is 7d, 30d or 90d")),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    Ok(quod_ledger::format_rfc3339(
        now.saturating_sub(days * 86_400),
        0,
    ))
}

async fn overview(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(team): Path<String>,
    Query(q): Query<RangeQ>,
) -> Result<Json, ApiError> {
    let range = q.range.unwrap_or_else(|| "7d".into());
    let from = since(&range)?;
    with_team(app, headers, team, move |_, d| {
        let rows: Vec<&StepRow> = d.rows.iter().filter(|r| r.at >= from).collect();
        let saved = minutes(rows.iter().filter_map(|r| r.saved_ms).sum());
        let (spot, identical, wrong) = spot_checks(d, Some(&from));
        // One point per UTC day of the range, zeros included, oldest first.
        let mut days: BTreeMap<String, (u64, u64)> = day_range(&range)
            .into_iter()
            .map(|d| (d, (0, 0)))
            .collect();
        for r in &rows {
            let e = days.entry(r.at[..10].to_string()).or_default();
            if replayed(r) {
                e.0 += r.recorded_ms.unwrap_or(0);
            } else {
                e.1 += r.wall_ms.unwrap_or(0);
            }
        }
        let states: Vec<Value> = d
            .trust
            .keys()
            .map(|(r, k)| d.trust_state(r, k))
            .collect();
        let count = |k: &str| states.iter().filter(|s| s["kind"] == k).count();
        Ok(json!({
            "asOf": now_rfc3339(),
            "totals": {
                "range": range,
                "savedMinutes": saved,
                "stepsReplayed": rows.iter().filter(|r| replayed(r)).count(),
                "stepsTotal": rows.len(),
                "costSavedUsd": saved as f64 * USD_PER_MINUTE,
                "usdPerMinute": USD_PER_MINUTE,
                "wrongReplays": wrong,
                "spotChecks": spot,
                "spotChecksIdentical": identical,
            },
            "days": days.into_iter().map(|(day, (rep, ran))| json!({"day": day, "replayedMinutes": rep as f64 / 60_000.0, "ranMinutes": ran as f64 / 60_000.0})).collect::<Vec<_>>(),
            "repos": d.repos.iter().map(|r| repo_summary(d, r)).collect::<Vec<_>>(),
            "trust": {
                "proven": count("replaying") + count("watching"),
                "proving": count("proving"),
                "watching": count("watching"),
                "spotChecksPassed": identical,
                "paused": paused_refs(d),
            },
            "runners": [],
        }))
    })
    .await
}

/// The UTC days of a range ending today, oldest first.
fn day_range(range: &str) -> Vec<String> {
    let n: u64 = match range {
        "30d" => 30,
        "90d" => 90,
        _ => 7,
    };
    let today = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 86_400;
    (0..n)
        .rev()
        .map(|i| quod_ledger::format_rfc3339((today - i) * 86_400, 0)[..10].to_string())
        .collect()
}

async fn repos(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(team): Path<String>,
) -> Result<Json, ApiError> {
    with_team(app, headers, team, |_, d| {
        Ok(json!({"repos": d.repos.iter().map(|r| repo_summary(d, r)).collect::<Vec<_>>()}))
    })
    .await
}

async fn repo(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((team, slug)): Path<(String, String)>,
) -> Result<Json, ApiError> {
    with_team(app, headers, team, move |_, d| {
        let repo = d
            .repo_by_slug(&slug)
            .ok_or_else(|| ApiError::not_found("no such repo"))?;
        let runs = runs_of(d, Some(&repo));
        let latest = runs.last().map(|g| run_detail(d, g));
        let times: Vec<Value> = runs
            .iter()
            .rev()
            .take(30)
            .rev()
            .map(|g| {
                let s = run_summary(d, g);
                json!({"number": g.number, "withQuodSeconds": s["tookSeconds"], "withoutSeconds": s["wouldHaveTakenSeconds"]})
            })
            .collect();
        let default_branch = d
            .team
            .as_ref()
            .map_or_else(|| "main".to_string(), |t| t.default_branch(&repo));
        Ok(json!({
            "asOf": now_rfc3339(),
            "summary": repo_summary(d, &repo),
            "defaultBranch": default_branch,
            "requireProtectedRef": d.require_protected_ref(Some(&repo)),
            "latestRun": latest,
            "runTimes": times,
            "steps": step_summaries(d, Some(&repo)),
        }))
    })
    .await
}

#[derive(Deserialize)]
struct RunsQ {
    repo: Option<String>,
    limit: Option<usize>,
}

async fn runs(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(team): Path<String>,
    Query(q): Query<RunsQ>,
) -> Result<Json, ApiError> {
    with_team(app, headers, team, move |_, d| {
        let repo = match &q.repo {
            Some(s) => Some(
                d.repo_by_slug(s)
                    .ok_or_else(|| ApiError::not_found("no such repo"))?,
            ),
            None => None,
        };
        let mut all: Vec<(String, Value)> = runs_of(d, repo.as_deref())
            .iter()
            .map(|g| (g.rows[0].at.clone(), run_summary(d, g)))
            .collect();
        all.sort_by(|a, b| b.0.cmp(&a.0));
        let limit = q.limit.unwrap_or(50).min(500);
        Ok(json!({"asOf": now_rfc3339(), "runs": all.into_iter().take(limit).map(|x| x.1).collect::<Vec<_>>()}))
    })
    .await
}

async fn run(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((team, slug, number)): Path<(String, String, u64)>,
) -> Result<Json, ApiError> {
    with_team(app, headers, team, move |_, d| {
        let repo = d
            .repo_by_slug(&slug)
            .ok_or_else(|| ApiError::not_found("no such repo"))?;
        // The newest run (and attempt) with that number.
        let runs = runs_of(d, Some(&repo));
        let g = runs
            .iter()
            .rev()
            .find(|g| g.number == number)
            .ok_or_else(|| ApiError::not_found("no such run"))?;
        Ok(run_detail(d, g))
    })
    .await
}

#[derive(Deserialize)]
struct RepoQ {
    repo: Option<String>,
}

async fn steps(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(team): Path<String>,
    Query(q): Query<RepoQ>,
) -> Result<Json, ApiError> {
    with_team(app, headers, team, move |_, d| {
        let repo = match &q.repo {
            Some(s) => Some(
                d.repo_by_slug(s)
                    .ok_or_else(|| ApiError::not_found("no such repo"))?,
            ),
            None => None,
        };
        Ok(json!({"steps": step_summaries(d, repo.as_deref())}))
    })
    .await
}

async fn step(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((team, slug, key)): Path<(String, String, String)>,
) -> Result<Json, ApiError> {
    with_team(app, headers, team, move |app, d| {
        let repo = d
            .repo_by_slug(&slug)
            .ok_or_else(|| ApiError::not_found("no such repo"))?;
        let key = crate::blobs::parse(&key)
            .map_err(|_| ApiError::not_found("no such step"))?
            .hex();
        let rows: Vec<&StepRow> = d
            .rows
            .iter()
            .filter(|r| r.repository == repo && key_hex(r) == key)
            .collect();
        if rows.is_empty() && !d.trust.contains_key(&(repo.clone(), key.clone())) {
            return Err(ApiError::not_found("no such step"));
        }
        let mut v = if rows.is_empty() {
            json!({"repoSlug": d.slug(&repo), "key": key, "job": "", "command": "", "trust": d.trust_state(&repo, &key), "replayRate": null, "ranSeconds": 0, "replaySeconds": null, "savedMinutes": 0})
        } else {
            step_summary(d, &repo, &key, &rows)
        };
        v["asOf"] = json!(now_rfc3339());
        v["proofsNeeded"] = json!(d.needed(&repo, &key));
        // The newest divergence, as the audit log has it.
        let div = d
            .audits
            .iter()
            .chain(&d.branch_audits)
            .filter(|(r, k, a)| *r == repo && *k == key && a.diverged)
            .max_by_key(|(_, _, a)| a.id)
            .map(|(_, _, a)| {
                let num = |run: &Option<String>| run.as_deref().map_or(0, |s| d.source_run_number(s));
                json!({
                    "recordedRun": num(&a.expected_run), "recheckRun": num(&a.actual_run),
                    "inputs": rows.last().and_then(|r| r.inputs).unwrap_or(0),
                    "stream": "stdout", "line": 0, "recorded": [],
                    "recheck": a.detail.iter().cloned().collect::<Vec<_>>(),
                    "exitCode": {"recorded": 0, "recheck": 0},
                })
            });
        v["divergence"] = div.unwrap_or(Value::Null);
        let mut history = vec![];
        if let Some(first) = rows.first() {
            history.push(json!({"kind": "first-recorded", "at": first.at, "run": first.run_number.unwrap_or(0), "seconds": secs(first.wall_ms)}));
        }
        for (_, _, a) in d
            .audits
            .iter()
            .chain(&d.branch_audits)
            .filter(|(r, k, _)| *r == repo && *k == key)
        {
            let run = a.actual_run.as_deref().map_or(0, |s| d.source_run_number(s));
            history.push(if a.diverged {
                json!({"kind": "paused", "at": a.at, "run": run})
            } else if a.phase == "audit" {
                json!({"kind": "spot-check-passed", "at": a.at, "run": run})
            } else {
                let same = a.expected_run.as_deref().map_or(0, |s| d.source_run_number(s));
                json!({"kind": "consistent", "at": a.at, "run": run, "sameAs": same})
            });
        }
        let reps: Vec<&&StepRow> = rows.iter().filter(|r| replayed(r)).collect();
        if let Some(last) = reps.last() {
            history.push(json!({"kind": "replayed", "at": last.at, "times": reps.len(), "savedMinutes": minutes(reps.iter().filter_map(|r| r.saved_ms).sum())}));
        }
        history.sort_by(|a, b| b["at"].as_str().cmp(&a["at"].as_str()));
        v["history"] = Value::Array(history);
        v["inputs"] = inputs(app, &d.id, &repo, rows.last().copied())?;
        Ok(v)
    })
    .await
}

/// Input counts of a step's newest manifest.
fn inputs(app: &App, team: &str, repo: &str, row: Option<&StepRow>) -> Result<Value, ApiError> {
    let mut counts =
        json!({"filesRead": 0, "foldersListed": 0, "pathsNotFound": 0, "networkAccess": 0});
    let Some(mid) = row
        .and_then(|r| r.manifest_id.as_deref())
        .and_then(|m| m.parse::<Hash>().ok())
    else {
        return Ok(counts);
    };
    let scope = &row.unwrap().scope;
    let Some(l) = app.store.ledger_by_name(team, repo, scope)? else {
        return Ok(counts);
    };
    if let Some(m) = l.manifest(&mid)? {
        let n = |k: Kind| m.entries.iter().filter(|e| e.k == k).count();
        counts["filesRead"] = json!(n(Kind::File) + n(Kind::Symlink));
        counts["foldersListed"] = json!(n(Kind::Dir));
        counts["pathsNotFound"] = json!(n(Kind::Absent));
    }
    Ok(counts)
}

async fn search_index(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(team): Path<String>,
) -> Result<Json, ApiError> {
    with_team(app, headers, team, |_, d| {
        let runs: Vec<Value> = runs_of(d, None)
            .iter()
            .rev()
            .take(100)
            .map(|g| {
                let s = run_summary(d, g);
                json!({"repoSlug": s["repoSlug"], "number": g.number, "title": s["title"], "status": s["status"]})
            })
            .collect();
        let steps: Vec<Value> = step_summaries(d, None)
            .into_iter()
            .map(|s| json!({"repoSlug": s["repoSlug"], "key": s["key"], "command": s["command"], "trust": s["trust"]["kind"]}))
            .collect();
        Ok(json!({
            "repos": d.repos.iter().map(|r| json!({"slug": d.slug(r), "fullName": r})).collect::<Vec<_>>(),
            "runs": runs,
            "steps": steps,
        }))
    })
    .await
}

async fn activity(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(team): Path<String>,
) -> Result<Json, ApiError> {
    blocking(move || {
        let v = viewer(&app, &headers, &team)?;
        let last = app.store.activity(&v.team)?.last_event()?;
        // Live job status (queued, running) comes from the GitHub App's
        // tables, which the dashboard reads itself (cloud/docs/api.md);
        // quod-server has step events, not jobs.
        Ok(axum::Json(json!({
            "asOf": now_rfc3339(),
            "jobs": [],
            "cursor": last.to_string(),
            "simulated": false,
        })))
    })
    .await
}

async fn rerun() -> ApiError {
    ApiError::not_implemented(
        "re-running a workflow without Quod needs the GitHub App's actions:write, which it does not have (P5)",
    )
}

async fn set_proofs(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((team, _, _)): Path<(String, String, String)>,
) -> Result<Json, ApiError> {
    blocking(move || {
        admin(&app, &headers, &team)?;
        Err(ApiError::not_implemented(
            "per-step proofs are not stored yet; the team policy sets them (P5)",
        ))
    })
    .await
}

async fn runners() -> ApiError {
    ApiError::not_implemented(
        "runners come from the GitHub App's job data, which the dashboard reads itself",
    )
}

async fn connect() -> ApiError {
    ApiError::not_implemented("the Connect setup is not stored by quod-server yet (P5)")
}

async fn save_connect(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(team): Path<String>,
) -> Result<Json, ApiError> {
    blocking(move || {
        admin(&app, &headers, &team)?;
        Err(ApiError::not_implemented(
            "the Connect choice is not stored by quod-server yet (P5)",
        ))
    })
    .await
}

// ----- events -----

/// The `step.finished` LiveEvent of a step row (its id and type are added
/// as it is sent).
pub fn step_finished_event(
    app: &App,
    team: &str,
    row: &StepRow,
    _state: Option<TrustState>,
) -> Value {
    let d = TeamData::for_row(app, team, row).unwrap_or_else(|_| TeamData {
        rows: vec![],
        repos: [row.repository.clone()].into(),
        trust: BTreeMap::new(),
        trust_ref: BTreeMap::new(),
        streaks: BTreeMap::new(),
        audits: vec![],
        branch_audits: vec![],
        recent_div: BTreeMap::new(),
        proofs: quod_trust::DEFAULT_PROOFS,
        writes: vec![],
        team: None,
        id: team.to_string(),
    });
    json!({
        "at": row.at,
        "repoSlug": d.slug(&row.repository),
        "run": row.run_number.unwrap_or(0),
        "runId": row.run_id.as_deref().and_then(|s| s.parse::<u64>().ok()),
        "runAttempt": row.run_attempt,
        "jobId": job_id(row),
        "index": row.step_index.unwrap_or(0),
        "stepKey": key_or_null(row),
        "step": run_step(&d, row),
    })
}

/// The `trust.changed` LiveEvent of a verdict.
pub fn trust_changed_event(
    app: &App,
    team: &str,
    repository: &str,
    key: &Hash,
    v: &Verdict,
) -> Value {
    let repos: BTreeSet<String> = app
        .store
        .scopes(team)
        .into_iter()
        .map(|s| s.repository)
        .chain([repository.to_string()])
        .collect();
    let at = now_rfc3339();
    let (trust, event) = if v.divergence.is_some() {
        (
            json!({"kind": "paused", "consistent": 0, "need": v.needed}),
            json!({"kind": "paused", "at": at, "run": 0}),
        )
    } else {
        (
            json!({"kind": "replaying"}),
            json!({"kind": "proven", "at": at, "runs": [0, 0]}),
        )
    };
    json!({"at": at, "repoSlug": slug(repository, &repos), "key": key.hex(), "trust": trust, "event": event})
}

#[derive(Deserialize)]
struct EventsQ {
    scope: Option<String>,
    repo: Option<String>,
    run: Option<u64>,
    key: Option<String>,
}

fn wants(q: &EventsQ, repo_of: &dyn Fn(&str) -> String, e: &EventRow) -> bool {
    let repo_ok = || {
        q.repo
            .as_deref()
            .is_none_or(|s| e.repository.as_deref().is_some_and(|r| repo_of(r) == s))
    };
    match q.scope.as_deref().unwrap_or("team") {
        "repo" => repo_ok(),
        "run" => repo_ok() && q.run.is_some() && e.run_number == q.run,
        "step" => {
            repo_ok()
                && q.key.as_deref().is_some_and(|k| {
                    e.step_key
                        .as_deref()
                        .and_then(|s| s.parse::<Hash>().ok())
                        .is_some_and(|h| h.hex() == k.trim_start_matches("b3:"))
                })
        }
        _ => true,
    }
}

async fn events(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(team): Path<String>,
    Query(q): Query<EventsQ>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let after: i64 = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(-1);
    let (a2, h2, t2) = (app.clone(), headers.clone(), team.clone());
    let (start, team) = blocking(move || {
        let v = viewer(&a2, &h2, &t2)?;
        // Without Last-Event-ID the stream starts now.
        let start = if after >= 0 {
            after
        } else {
            a2.store.activity(&v.team)?.last_event()?
        };
        Ok((start, v.team))
    })
    .await?;
    let hold = app.settings.sse_hold;
    let (tx, rx) = tokio::sync::mpsc::channel::<Event>(64);
    tokio::spawn(async move {
        let mut last = start;
        let begun = tokio::time::Instant::now();
        while begun.elapsed() < hold {
            let (a, t) = (app.clone(), team.clone());
            let batch = blocking(move || {
                let act = a.store.activity(&t)?;
                let ev = act.events_after(last, 500)?;
                let repos: BTreeSet<String> = a
                    .store
                    .scopes(&t)
                    .into_iter()
                    .map(|s| s.repository)
                    .collect();
                Ok((ev, repos))
            })
            .await;
            let Ok((batch, repos)) = batch else { break };
            let repo_of = |r: &str| slug(r, &repos);
            for e in batch {
                last = e.id;
                if !wants(&q, &repo_of, &e) {
                    continue;
                }
                let mut data = e.json.clone();
                data["id"] = json!(e.id.to_string());
                data["type"] = json!(e.kind);
                let ev = Event::default()
                    .id(e.id.to_string())
                    .event(e.kind.clone())
                    .data(data.to_string());
                if tx.send(ev).await.is_err() {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let _ = tx.send(Event::default().event("bye").data("{}")).await;
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|e| (Ok::<_, Infallible>(e), rx))
    });
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}
