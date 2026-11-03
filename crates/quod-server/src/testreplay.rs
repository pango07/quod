//! Test-level replay (`quod vitest`) in Quod Cloud: ledger directories
//! stored per repository, suite, scope and commit, and the per-run reports
//! the dashboard shows. quod-wire's `testreplay` has the protocol;
//! docs/test-replay-cloud.md the design.
//!
//! Trust. A ledger decides which test results a later run reuses without
//! running them, so whoever can write a ledger a job reads can hide a
//! failure from that job. The writer model is the step ledger's
//! (`auth::oidc_scopes`):
//! - a writer job (push, dispatch or schedule on a writer ref) saves into
//!   `trusted`, the default branch's ledgers, and reads only those;
//! - the repository's other jobs (another branch, a same-repository pull
//!   request, a merge queue) save into their ref's own scope and read it
//!   first, then `trusted`;
//! - jobs on someone else's input (`unverified`: pull_request_target,
//!   workflow_run, ...) and fork scopes read `trusted` and never save.
//!   Fork pull requests get no OIDC token from GitHub at all.
//! Rows are only ever added: nothing a non-writer sends can change or
//! replace a `trusted` row, and a lookup reads only the scopes above.
//!
//! Storage: `teams/<team>/test-replay.sqlite`, its own file, so
//! `team.sqlite`'s schema (and a rollback to a build without this module)
//! is untouched. File bytes live in the team's CAS like every other blob.

use std::collections::BTreeSet;
use std::path::Path;

use axum::Extension;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use quod_cas::BlobStore;
use quod_ledger::now_rfc3339;
use quod_wire::testreplay::*;
use quod_wire::{CiContext, PROTOCOL};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{Principal, Runner, TRUSTED, UNVERIFIED};
use crate::{ApiError, App, AppState, blocking};

/// `test-replay.sqlite`'s schema version.
pub const VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS ledgers (
  id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, repo_key TEXT NOT NULL, repository TEXT NOT NULL,
  suite TEXT NOT NULL, scope TEXT NOT NULL, git_ref TEXT, sha TEXT, files TEXT NOT NULL, files_hash TEXT NOT NULL,
  n_files INTEGER NOT NULL, bytes INTEGER NOT NULL, run_id TEXT, workflow TEXT, job TEXT);
CREATE INDEX IF NOT EXISTS ledgers_by_scope ON ledgers (repo_key, suite, scope, id);
CREATE INDEX IF NOT EXISTS ledgers_by_sha ON ledgers (repo_key, suite, sha, id);
CREATE TABLE IF NOT EXISTS runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, repo_key TEXT NOT NULL, repository TEXT NOT NULL,
  suite TEXT NOT NULL, scope TEXT NOT NULL, kind TEXT NOT NULL, tool TEXT NOT NULL,
  run_id TEXT, run_attempt INTEGER, run_number INTEGER, workflow TEXT, job TEXT, job_name TEXT,
  git_ref TEXT, sha TEXT, event TEXT, actor TEXT,
  total INTEGER NOT NULL, ran INTEGER NOT NULL, replayed INTEGER NOT NULL, narrowed INTEGER NOT NULL,
  wall_ms REAL, select_ms REAL, est_plain_ms REAL, saved_ms REAL, exit INTEGER,
  audited INTEGER NOT NULL, audit_mismatches INTEGER NOT NULL, detail TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS runs_by_at ON runs (at);
CREATE INDEX IF NOT EXISTS runs_by_commit ON runs (repo_key, suite, sha, kind, id);
";

/// A team's test-replay database.
pub struct Db {
    db: Connection,
}

impl Db {
    pub fn open(path: &Path) -> anyhow::Result<Db> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(30))?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        let v: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if v > VERSION {
            anyhow::bail!(
                "{} has schema version {v}, newer than this server's {VERSION}",
                path.display()
            );
        }
        if v < VERSION {
            db.execute_batch("BEGIN IMMEDIATE")?;
            let r = db
                .execute_batch(SCHEMA)
                .and_then(|_| db.pragma_update(None, "user_version", VERSION));
            match r {
                Ok(()) => db.execute_batch("COMMIT")?,
                Err(e) => {
                    let _ = db.execute_batch("ROLLBACK");
                    return Err(e.into());
                }
            }
        }
        Ok(Db { db })
    }
}

fn db(app: &App, team: &str) -> Result<Db, ApiError> {
    Ok(Db::open(
        &app.store.team_dir(team).join("test-replay.sqlite"),
    )?)
}

/// Where a runner's repository is keyed: GitHub's id when known (a
/// rename keeps the ledgers, a new repository under an old name starts
/// empty), else its name.
fn repo_key(r: &Runner) -> Result<String, ApiError> {
    if r.repository.is_empty() {
        return Err(ApiError::forbidden("this runner token names no repository"));
    }
    Ok(match r.repository_id {
        Some(id) => format!("github-{id}"),
        None => format!("name-{}", r.repository),
    })
}

/// Whether a job may save a ledger: not one on someone else's input.
fn may_save(r: &Runner) -> bool {
    r.write != UNVERIFIED && !r.write.starts_with("fork-")
}

/// The scopes a job's lookup reads, in order: its own (unless it may not
/// save), then the default branch's (`trusted`).
pub fn read_scopes(r: &Runner) -> Vec<String> {
    let mut v = vec![];
    if may_save(r) && r.write != TRUSTED {
        v.push(r.write.clone());
    }
    v.push(TRUSTED.to_string());
    v
}

/// The job's facts: OIDC claims win over what the client says.
fn ci(r: &Runner, client: &Option<CiContext>) -> CiContext {
    let c = client.clone().unwrap_or_default();
    match &r.claims {
        Some(cl) => CiContext {
            job: c.job,
            job_id: c.job_id,
            job_name: c.job_name,
            step_index: c.step_index,
            step_name: c.step_name,
            runner: c.runner,
            ..cl.clone()
        },
        None => c,
    }
}

fn sha_of(c: &CiContext) -> Option<String> {
    c.sha.clone().filter(|s| valid_sha(s))
}

fn check_suite(s: &str) -> Result<(), ApiError> {
    if !valid_suite(s) {
        return Err(ApiError::bad_request("invalid suite"));
    }
    Ok(())
}

struct Row {
    id: i64,
    at: String,
    scope: String,
    git_ref: Option<String>,
    sha: Option<String>,
    files: String,
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        at: r.get(1)?,
        scope: r.get(2)?,
        git_ref: r.get(3)?,
        sha: r.get(4)?,
        files: r.get(5)?,
    })
}

const COLS: &str = "id, at, scope, git_ref, sha, files";

/// The ledger a lookup answers with: the newest of the first ancestor
/// commit any read scope has (the job's own scope first), else the newest
/// of the job's own scope, else the newest of `trusted`.
fn find(
    db: &Db,
    repo: &str,
    suite: &str,
    scopes: &[String],
    ancestors: &[String],
) -> anyhow::Result<Option<(Row, &'static str)>> {
    let own = scopes.first().cloned().unwrap_or_default();
    let in_scopes = |s: &str| scopes.iter().any(|x| x == s);
    let mut st = db.db.prepare(&format!(
        "SELECT {COLS} FROM ledgers WHERE repo_key = ?1 AND suite = ?2 AND sha = ?3 ORDER BY id DESC LIMIT 20"
    ))?;
    for sha in ancestors.iter().filter(|s| valid_sha(s)) {
        let rows: Vec<Row> = st
            .query_map(params![repo, suite, sha], row)?
            .collect::<Result<_, _>>()?;
        let pick = rows
            .iter()
            .position(|r| r.scope == own)
            .or_else(|| rows.iter().position(|r| in_scopes(&r.scope)));
        if let Some(i) = pick {
            return Ok(rows.into_iter().nth(i).map(|r| (r, "ancestor")));
        }
    }
    let mut st = db.db.prepare(&format!(
        "SELECT {COLS} FROM ledgers WHERE repo_key = ?1 AND suite = ?2 AND scope = ?3 ORDER BY id DESC LIMIT 1"
    ))?;
    for s in scopes {
        if let Some(r) = st.query_row(params![repo, suite, s], row).optional()? {
            let how = if s == TRUSTED { "default" } else { "branch" };
            return Ok(Some((r, how)));
        }
    }
    Ok(None)
}

pub async fn lookup(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<LookupRequest>,
) -> Result<Json<LookupResponse>, ApiError> {
    blocking(move || {
        let runner = App::runner_of(&principal, req.scope.as_ref())?;
        check_suite(&req.suite)?;
        if req.ancestors.len() > MAX_ANCESTORS || req.nonce.is_empty() || req.nonce.len() > 200 {
            return Err(ApiError::bad_request("too many ancestors, or a bad nonce"));
        }
        let repo = repo_key(&runner)?;
        let scopes = read_scopes(&runner);
        let d = db(&app, &runner.team)?;
        let found = find(&d, &repo, &req.suite, &scopes, &req.ancestors)?;
        let ledger = match found {
            None => None,
            Some((r, matched)) => {
                let files: Vec<LedgerFile> =
                    serde_json::from_str(&r.files).map_err(ApiError::internal)?;
                let statement = LedgerStatement {
                    protocol: PROTOCOL.into(),
                    kind: "test-ledger".into(),
                    team: runner.team.clone(),
                    repository: runner.repository.clone(),
                    scope: r.scope.clone(),
                    suite: req.suite.clone(),
                    ledger_id: r.id,
                    sha: r.sha.clone(),
                    files: files_hash(&files).map_err(ApiError::internal)?,
                    issued_at: now_rfc3339(),
                    nonce: req.nonce.clone(),
                };
                let receipt = sign_ledger(&app.settings.signing_key, statement)
                    .map_err(ApiError::internal)?;
                Some(FoundLedger {
                    id: r.id,
                    scope: r.scope,
                    git_ref: r.git_ref,
                    sha: r.sha,
                    at: r.at,
                    matched: matched.into(),
                    files,
                    receipt,
                })
            }
        };
        Ok(Json(LookupResponse {
            ledger,
            write_scope: may_save(&runner).then(|| runner.write.clone()),
        }))
    })
    .await
}

pub async fn save(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<SaveRequest>,
) -> Result<Json<SaveResponse>, ApiError> {
    blocking(move || {
        let runner = App::runner_of(&principal, req.scope.as_ref())?;
        check_suite(&req.suite)?;
        if !may_save(&runner) {
            return Err(ApiError::forbidden(
                "this job runs on input someone else controls; it may read the default branch's test ledger but not save one",
            ));
        }
        check_files(&req.files).map_err(ApiError::bad_request)?;
        let repo = repo_key(&runner)?;
        let cas = app.store.cas(&runner.team)?;
        let mut missing = vec![];
        for f in &req.files {
            let p = cas.blob_path(&f.hash);
            match std::fs::metadata(&p) {
                Ok(m) if m.len() == f.size => {}
                Ok(_) => {
                    return Err(ApiError::bad_request(format!(
                        "{} is listed with another size than its blob",
                        f.path
                    )));
                }
                Err(_) if !cas.has_blob(&f.hash).map_err(ApiError::internal)? => {
                    missing.push(f.hash)
                }
                Err(e) => return Err(ApiError::internal(e)),
            }
        }
        if !missing.is_empty() {
            missing.sort();
            missing.dedup();
            return Err(ApiError::missing_blobs(missing));
        }
        let c = ci(&runner, &req.ci);
        let mut files = req.files.clone();
        files.sort();
        let fh = files_hash(&files).map_err(ApiError::internal)?;
        let bytes: u64 = files.iter().map(|f| f.size).sum();
        let d = db(&app, &runner.team)?;
        d.db.execute(
            "INSERT INTO ledgers (at, repo_key, repository, suite, scope, git_ref, sha, files, files_hash, n_files, bytes, run_id, workflow, job)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                now_rfc3339(), repo, runner.repository, req.suite, runner.write,
                c.git_ref.as_deref().map(crate::api::cap), sha_of(&c),
                serde_json::to_string(&files).map_err(ApiError::internal)?, fh.hex(),
                files.len() as i64, bytes as i64,
                c.run_id.as_deref().map(crate::api::cap), c.workflow.as_deref().map(crate::api::cap),
                c.job.as_deref().map(crate::api::cap),
            ],
        )
        .map_err(ApiError::internal)?;
        let id = d.db.last_insert_rowid();
        tracing::info!(target: "quod", team = %runner.team, repository = %runner.repository, scope = %runner.write, suite = %req.suite, files = files.len(), bytes, "test ledger saved");
        let notice = crate::api::note_trusted_write_ref(
            &app,
            &runner,
            c.git_ref.as_deref().unwrap_or_default(),
        );
        Ok(Json(SaveResponse {
            id,
            scope: runner.write.clone(),
            notice,
        }))
    })
    .await
}

/// `POST /v1/blobs/pack`: the blobs asked for, in order, in one answer
/// (`PackRequest`). Team-wide, as `GET /v1/blobs/{hash}` is.
pub async fn pack(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<PackRequest>,
) -> Result<Response, ApiError> {
    let team = App::runner_team_of(&principal)?;
    if req.hashes.len() > MAX_PACK {
        return Err(ApiError::bad_request("too many hashes"));
    }
    let cas = {
        let a = app.clone();
        let t = team.clone();
        blocking(move || Ok(a.store.cas(&t)?)).await?
    };
    let dir = std::sync::Arc::new(cas);
    let stream = futures_util::stream::iter(req.hashes).then(move |h| {
        let dir = dir.clone();
        async move {
            let p = dir.blob_path(&h);
            let mut out = match tokio::fs::read(&p).await {
                Ok(b) => {
                    let mut o = format!("{} {}\n", h.hex(), b.len()).into_bytes();
                    o.extend_from_slice(&b);
                    o
                }
                Err(_) => format!("{} -\n", h.hex()).into_bytes(),
            };
            out.shrink_to_fit();
            Ok::<_, std::io::Error>(axum::body::Bytes::from(out))
        }
    });
    Ok((
        [("content-type", "application/octet-stream")],
        Body::from_stream(stream),
    )
        .into_response())
}

/// A report's text, capped.
fn capped(s: &str, n: usize) -> String {
    let mut end = s.len().min(n);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

fn finite(v: Option<f64>) -> Option<f64> {
    v.filter(|x| x.is_finite() && *x >= 0.0 && *x < 1e12)
}

pub async fn report(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<ReportRequest>,
) -> Result<Json<ReportResponse>, ApiError> {
    blocking(move || {
        let runner = App::runner_of(&principal, req.scope.as_ref())?;
        check_suite(&req.suite)?;
        let r = &req.report;
        if !matches!(r.kind.as_str(), "critical" | "shadow") || r.tool != "vitest" {
            return Err(ApiError::bad_request("kind is critical or shadow; tool is vitest"));
        }
        if r.ran_files.len() > MAX_REPORT_FILES || r.audit_files.len() > MAX_REPORT_FILES {
            return Err(ApiError::bad_request("too many files in the report"));
        }
        let max = 10_000_000u64;
        if [r.total, r.ran, r.replayed, r.narrowed, r.audited]
            .iter()
            .any(|v| *v > max)
        {
            return Err(ApiError::bad_request("a count is out of range"));
        }
        let repo = repo_key(&runner)?;
        let c = ci(&runner, &req.ci);
        let ran_files: Vec<RanFile> = r
            .ran_files
            .iter()
            .map(|f| RanFile {
                file: capped(&f.file, 1024),
                project: capped(&f.project, 200),
                why: capped(&f.why, 1024),
                duration_ms: finite(f.duration_ms),
                state: capped(&f.state, 40),
            })
            .collect();
        let audit_files: Vec<AuditFile> = r
            .audit_files
            .iter()
            .map(|f| AuditFile {
                file: capped(&f.file, 1024),
                ok: f.ok,
            })
            .collect();
        let mismatches = audit_files.iter().filter(|f| !f.ok).count() as i64;
        let ledger = r.ledger.as_ref().map(|l| LedgerUsed {
            found: l.found,
            id: l.id,
            sha: l.sha.as_deref().filter(|s| valid_sha(s)).map(str::to_string),
            git_ref: l.git_ref.as_deref().map(|s| capped(s, 300)),
            scope: l.scope.as_deref().map(|s| capped(s, 200)),
            matched: l.matched.as_deref().map(|s| capped(s, 40)),
        });
        let detail = json!({ "ranFiles": ran_files, "auditFiles": audit_files, "ledger": ledger });
        let cap = |s: &Option<String>| s.as_deref().map(crate::api::cap);
        let d = db(&app, &runner.team)?;
        d.db.execute(
            "INSERT INTO runs (at, repo_key, repository, suite, scope, kind, tool, run_id, run_attempt, run_number, workflow, job, job_name, git_ref, sha, event, actor,
               total, ran, replayed, narrowed, wall_ms, select_ms, est_plain_ms, saved_ms, exit, audited, audit_mismatches, detail)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29)",
            params![
                now_rfc3339(), repo, runner.repository, req.suite, runner.write, r.kind, r.tool,
                cap(&c.run_id), c.run_attempt.map(|v| v as i64), c.run_number.map(|v| v as i64),
                cap(&c.workflow), cap(&c.job), cap(&c.job_name), cap(&c.git_ref), sha_of(&c), cap(&c.event), cap(&c.actor),
                r.total as i64, r.ran as i64, r.replayed as i64, r.narrowed as i64,
                finite(r.wall_ms), finite(r.select_ms), finite(r.est_plain_ms), finite(r.saved_ms), r.exit,
                r.audited as i64, mismatches, detail.to_string(),
            ],
        )
        .map_err(ApiError::internal)?;
        let id = d.db.last_insert_rowid();
        // The live event: the summary as the dashboard lists it.
        let out = (|| -> anyhow::Result<()> {
            let repos = all_repos(&app, &runner.team, &d)?;
            let run = load_run(&d, id)?.ok_or_else(|| anyhow::anyhow!("row gone"))?;
            let summary = summary(&d, &run, &repos)?;
            app.store.activity(&runner.team)?.push_event(
                "test.run",
                Some(&runner.repository),
                c.run_number,
                None,
                &json!({"at": run.at, "repoSlug": summary["repoSlug"], "run": summary}),
            )?;
            Ok(())
        })();
        if let Err(e) = out {
            tracing::warn!(target: "quod", error = %format!("{e:#}"), "test run event not written");
        }
        Ok(Json(ReportResponse { id }))
    })
    .await
}

// ----- the dashboard's reads -----

#[derive(Clone, Debug)]
struct RunRow {
    id: i64,
    at: String,
    repo_key: String,
    repository: String,
    suite: String,
    scope: String,
    kind: String,
    tool: String,
    run_id: Option<String>,
    run_attempt: Option<i64>,
    run_number: Option<i64>,
    workflow: Option<String>,
    job: Option<String>,
    job_name: Option<String>,
    git_ref: Option<String>,
    sha: Option<String>,
    event: Option<String>,
    total: i64,
    ran: i64,
    replayed: i64,
    narrowed: i64,
    wall_ms: Option<f64>,
    select_ms: Option<f64>,
    est_plain_ms: Option<f64>,
    saved_ms: Option<f64>,
    exit: Option<i64>,
    audited: i64,
    audit_mismatches: i64,
    detail: String,
}

const RUN_COLS: &str = "id, at, repo_key, repository, suite, scope, kind, tool, run_id, run_attempt, run_number, workflow, job, job_name, git_ref, sha, event, total, ran, replayed, narrowed, wall_ms, select_ms, est_plain_ms, saved_ms, exit, audited, audit_mismatches, detail";

fn run_row(r: &rusqlite::Row) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: r.get(0)?,
        at: r.get(1)?,
        repo_key: r.get(2)?,
        repository: r.get(3)?,
        suite: r.get(4)?,
        scope: r.get(5)?,
        kind: r.get(6)?,
        tool: r.get(7)?,
        run_id: r.get(8)?,
        run_attempt: r.get(9)?,
        run_number: r.get(10)?,
        workflow: r.get(11)?,
        job: r.get(12)?,
        job_name: r.get(13)?,
        git_ref: r.get(14)?,
        sha: r.get(15)?,
        event: r.get(16)?,
        total: r.get(17)?,
        ran: r.get(18)?,
        replayed: r.get(19)?,
        narrowed: r.get(20)?,
        wall_ms: r.get(21)?,
        select_ms: r.get(22)?,
        est_plain_ms: r.get(23)?,
        saved_ms: r.get(24)?,
        exit: r.get(25)?,
        audited: r.get(26)?,
        audit_mismatches: r.get(27)?,
        detail: r.get(28)?,
    })
}

fn load_run(d: &Db, id: i64) -> anyhow::Result<Option<RunRow>> {
    Ok(d.db
        .query_row(
            &format!("SELECT {RUN_COLS} FROM runs WHERE id = ?1"),
            params![id],
            run_row,
        )
        .optional()?)
}

/// The shadow report of the same commit and suite as a critical one.
fn shadow_of(d: &Db, r: &RunRow) -> anyhow::Result<Option<RunRow>> {
    let Some(sha) = &r.sha else { return Ok(None) };
    Ok(d.db
        .query_row(
            &format!(
                "SELECT {RUN_COLS} FROM runs WHERE repo_key = ?1 AND suite = ?2 AND sha = ?3 AND kind = 'shadow' ORDER BY id DESC LIMIT 1"
            ),
            params![r.repo_key, r.suite, sha],
            run_row,
        )
        .optional()?)
}

/// Every repository name the team has (step scopes and test runs), for
/// dashboard slugs.
fn all_repos(app: &App, team: &str, d: &Db) -> anyhow::Result<BTreeSet<String>> {
    let mut s: BTreeSet<String> = app
        .store
        .scopes(team)
        .into_iter()
        .map(|s| s.repository)
        .collect();
    let mut st = d.db.prepare("SELECT DISTINCT repository FROM runs")?;
    for r in st.query_map([], |r| r.get::<_, String>(0))? {
        s.insert(r?);
    }
    Ok(s)
}

fn branch(r: Option<&str>) -> Option<String> {
    r.map(|r| {
        r.strip_prefix("refs/heads/")
            .or_else(|| r.strip_prefix("refs/tags/"))
            .unwrap_or(r)
            .to_string()
    })
}

fn base_summary(r: &RunRow, repos: &BTreeSet<String>) -> Value {
    let github_url = r
        .run_id
        .as_ref()
        .filter(|id| id.bytes().all(|b| b.is_ascii_digit()))
        .map(|id| match r.run_attempt {
            Some(a) => format!(
                "https://github.com/{}/actions/runs/{id}/attempts/{a}",
                r.repository
            ),
            None => format!("https://github.com/{}/actions/runs/{id}", r.repository),
        });
    let detail: Value = serde_json::from_str(&r.detail).unwrap_or(Value::Null);
    let ledger = detail.get("ledger").cloned().unwrap_or(Value::Null);
    json!({
        "id": r.id.to_string(),
        "at": r.at,
        "repoSlug": crate::dash::slug(&r.repository, repos),
        "repository": r.repository,
        "suite": r.suite,
        "kind": r.kind,
        "tool": r.tool,
        "workflow": r.workflow,
        "job": r.job_name.clone().or_else(|| r.job.clone()),
        "runId": r.run_id.as_deref().and_then(|s| s.parse::<u64>().ok()),
        "runAttempt": r.run_attempt,
        "runNumber": r.run_number,
        "githubUrl": github_url,
        "branch": branch(r.git_ref.as_deref()),
        "sha": r.sha,
        "event": r.event,
        "scope": r.scope,
        "files": {"total": r.total, "ran": r.ran, "replayed": r.replayed, "narrowed": r.narrowed},
        "timing": {"wallMs": r.wall_ms, "selectMs": r.select_ms, "estPlainMs": r.est_plain_ms, "savedMs": r.saved_ms},
        "exit": r.exit,
        "ledger": if ledger.is_null() { Value::Null } else { json!({
            "found": ledger["found"], "sha": ledger["sha"], "branch": branch(ledger["git_ref"].as_str()),
            "scope": ledger["scope"], "matched": ledger["matched"],
        }) },
        "audits": if r.kind == "shadow" { json!({"checked": r.audited, "mismatched": r.audit_mismatches}) } else { Value::Null },
    })
}

/// A run as the list shows it; a critical run carries its shadow's audits.
fn summary(d: &Db, r: &RunRow, repos: &BTreeSet<String>) -> anyhow::Result<Value> {
    let mut v = base_summary(r, repos);
    if r.kind == "critical"
        && let Some(s) = shadow_of(d, r)?
    {
        v["audits"] = json!({"checked": s.audited, "mismatched": s.audit_mismatches});
    }
    Ok(v)
}

#[derive(Deserialize)]
pub struct ListQ {
    repo: Option<String>,
    limit: Option<usize>,
}

pub async fn list_runs(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(team): UrlPath<String>,
    Query(q): Query<ListQ>,
) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let v = crate::dash::viewer(&app, &headers, &team)?;
        let d = db(&app, &v.team)?;
        let repos = all_repos(&app, &v.team, &d)?;
        let limit = q.limit.unwrap_or(50).clamp(1, 200);
        let mut st =
            d.db.prepare(&format!(
                "SELECT {RUN_COLS} FROM runs ORDER BY id DESC LIMIT 5000"
            ))
            .map_err(ApiError::internal)?;
        let rows: Vec<RunRow> = st
            .query_map([], run_row)
            .map_err(ApiError::internal)?
            .collect::<Result<_, _>>()
            .map_err(ApiError::internal)?;
        let mut out = vec![];
        for r in rows {
            if let Some(s) = &q.repo
                && crate::dash::slug(&r.repository, &repos) != *s
            {
                continue;
            }
            out.push(summary(&d, &r, &repos)?);
            if out.len() >= limit {
                break;
            }
        }
        Ok(Json(json!({ "runs": out })))
    })
    .await
}

pub async fn get_run(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath((team, id)): UrlPath<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let v = crate::dash::viewer(&app, &headers, &team)?;
        let id: i64 = id.parse().map_err(|_| ApiError::not_found("no such run"))?;
        let d = db(&app, &v.team)?;
        let r = load_run(&d, id)?.ok_or_else(|| ApiError::not_found("no such run"))?;
        let repos = all_repos(&app, &v.team, &d)?;
        let mut out = summary(&d, &r, &repos)?;
        let detail: Value = serde_json::from_str(&r.detail).unwrap_or(json!({}));
        let shadow = if r.kind == "critical" {
            shadow_of(&d, &r)?
        } else {
            None
        };
        let audit_source = shadow.as_ref().unwrap_or(&r);
        let audit_detail: Value = serde_json::from_str(&audit_source.detail).unwrap_or(json!({}));
        out["ranFiles"] = detail.get("ranFiles").cloned().unwrap_or(json!([]));
        out["auditFiles"] = audit_detail.get("auditFiles").cloned().unwrap_or(json!([]));
        out["shadow"] = match &shadow {
            Some(s) => base_summary(s, &repos),
            None => Value::Null,
        };
        Ok(Json(out))
    })
    .await
}

pub async fn stats(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(team): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let v = crate::dash::viewer(&app, &headers, &team)?;
        let d = db(&app, &v.team)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let today = quod_ledger::format_rfc3339(now / 86_400 * 86_400, 0);
        let week = quod_ledger::format_rfc3339(now.saturating_sub(7 * 86_400), 0);
        let sums = |from: &str| -> anyhow::Result<(f64, i64, i64, i64)> {
            Ok(d.db.query_row(
                "SELECT coalesce(sum(saved_ms), 0), count(*), coalesce(sum(total), 0), coalesce(sum(replayed), 0)
                 FROM runs WHERE kind = 'critical' AND at >= ?1",
                params![from],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        };
        let (t_saved, t_runs, _, _) = sums(&today)?;
        let (w_saved, w_runs, w_total, w_skipped) = sums(&week)?;
        let (checked, mismatched): (i64, i64) = d
            .db
            .query_row(
                "SELECT coalesce(sum(audited), 0), coalesce(sum(audit_mismatches), 0) FROM runs WHERE kind = 'shadow'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(ApiError::internal)?;
        let repos = all_repos(&app, &v.team, &d)?;
        let mut st = d
            .db
            .prepare(&format!(
                "SELECT {RUN_COLS} FROM runs WHERE kind = 'critical' ORDER BY id DESC LIMIT 10"
            ))
            .map_err(ApiError::internal)?;
        let recent: Vec<RunRow> = st
            .query_map([], run_row)
            .map_err(ApiError::internal)?
            .collect::<Result<_, _>>()
            .map_err(ApiError::internal)?;
        let recent: Vec<Value> = recent
            .iter()
            .map(|r| summary(&d, r, &repos))
            .collect::<anyhow::Result<_>>()?;
        Ok(Json(json!({
            "asOf": now_rfc3339(),
            "today": {"savedMs": t_saved, "runs": t_runs},
            "week": {
                "savedMs": w_saved, "runs": w_runs, "filesTotal": w_total, "filesSkipped": w_skipped,
                "skippedPct": if w_total > 0 { json!(100.0 * w_skipped as f64 / w_total as f64) } else { Value::Null },
            },
            "wrongResults": mismatched,
            "audits": {"checked": checked, "passed": checked - mismatched},
            "recent": recent,
        })))
    })
    .await
}

/// Every team's test-replay database, at start (as `Store::migrate`
/// does for the ledgers): a newer schema stops the boot.
pub fn migrate(store: &crate::store::Store) -> anyhow::Result<usize> {
    let mut n = 0;
    for team in store.teams_on_disk() {
        let p = store.team_dir(&team).join("test-replay.sqlite");
        if p.is_file() {
            Db::open(&p)?;
            n += 1;
        }
    }
    Ok(n)
}
