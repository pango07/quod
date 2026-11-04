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
//!
//! Rows are only ever added: nothing a non-writer sends can change or
//! replace a `trusted` row, and a lookup reads only the scopes above.
//!
//! Storage. Quod Cloud: the rows in Postgres (`test_ledgers`, `test_runs`,
//! `team` in every key and query), the files in the bucket with every
//! other blob. Self-hosted: `teams/<team>/test-replay.sqlite` and the
//! team's CAS. The tables are created idempotently at start, outside the
//! versioned migrations, so a rollback to a build without this module
//! finds a schema it knows.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use quod_cas::{BlobStore, Hash};
use quod_ledger::now_rfc3339;
use quod_wire::testreplay::*;
use quod_wire::{CiContext, PROTOCOL};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{Principal, Runner, TRUSTED, UNVERIFIED};
use crate::{ApiError, App, AppState, blocking};

/// The tables, in SQL both backends take (`{ID}` is the id column's
/// declaration, `{REAL}` a float column's).
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS test_ledgers (
  id {ID}, team TEXT NOT NULL, at TEXT NOT NULL, repo_key TEXT NOT NULL, repository TEXT NOT NULL,
  suite TEXT NOT NULL, scope TEXT NOT NULL, git_ref TEXT, sha TEXT, files TEXT NOT NULL, files_hash TEXT NOT NULL,
  n_files BIGINT NOT NULL, bytes BIGINT NOT NULL, run_id TEXT, workflow TEXT, job TEXT);
CREATE INDEX IF NOT EXISTS test_ledgers_by_scope ON test_ledgers (team, repo_key, suite, scope, id);
CREATE INDEX IF NOT EXISTS test_ledgers_by_sha ON test_ledgers (team, repo_key, suite, sha, id);
CREATE TABLE IF NOT EXISTS test_runs (
  id {ID}, team TEXT NOT NULL, at TEXT NOT NULL, repo_key TEXT NOT NULL, repository TEXT NOT NULL,
  suite TEXT NOT NULL, scope TEXT NOT NULL, kind TEXT NOT NULL, tool TEXT NOT NULL,
  run_id TEXT, run_attempt BIGINT, run_number BIGINT, workflow TEXT, job TEXT, job_name TEXT,
  git_ref TEXT, sha TEXT, event TEXT, actor TEXT,
  total BIGINT NOT NULL, ran BIGINT NOT NULL, replayed BIGINT NOT NULL, narrowed BIGINT NOT NULL,
  wall_ms {REAL}, select_ms {REAL}, est_plain_ms {REAL}, saved_ms {REAL}, exit BIGINT,
  audited BIGINT NOT NULL, audit_mismatches BIGINT NOT NULL, detail TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS test_runs_by_team ON test_runs (team, id);
CREATE INDEX IF NOT EXISTS test_runs_by_at ON test_runs (team, kind, at);
CREATE INDEX IF NOT EXISTS test_runs_by_commit ON test_runs (team, repo_key, suite, sha, kind, id);
CREATE TABLE IF NOT EXISTS test_bundles (
  team TEXT NOT NULL, ledger_id BIGINT NOT NULL, hash TEXT NOT NULL, size BIGINT NOT NULL,
  PRIMARY KEY (team, ledger_id));
CREATE TABLE IF NOT EXISTS test_file_stats (
  team TEXT NOT NULL, repo_key TEXT NOT NULL, suite TEXT NOT NULL, file TEXT NOT NULL,
  skipped BIGINT NOT NULL DEFAULT 0, saved_ms {REAL} NOT NULL DEFAULT 0,
  reason TEXT, reason_kind TEXT, last_at TEXT,
  PRIMARY KEY (team, repo_key, suite, file));
";

/// Creates the tables in Postgres (at start, from `boot::connect`).
pub fn ensure_pg(pg: &crate::pg::Pg) -> anyhow::Result<()> {
    let sql = SCHEMA
        .replace("{ID}", "BIGSERIAL PRIMARY KEY")
        .replace("{REAL}", "DOUBLE PRECISION")
        .replace("TEXT", "TEXT COLLATE \"C\"");
    let mut c = pg.conn()?;
    let mut tx = c.transaction()?;
    // One instance at a time (CREATE ... IF NOT EXISTS races otherwise).
    tx.execute("SELECT pg_advisory_xact_lock(7140513712)", &[])?;
    tx.batch_execute(&sql)?;
    tx.commit()?;
    Ok(())
}

/// A value bound to or read from a statement.
#[derive(Clone, Debug)]
pub(crate) enum V {
    I(Option<i64>),
    F(Option<f64>),
    T(Option<String>),
}

impl V {
    pub(crate) fn i(&self) -> Option<i64> {
        match self {
            V::I(v) => *v,
            _ => None,
        }
    }
    pub(crate) fn f(&self) -> Option<f64> {
        match self {
            V::F(v) => *v,
            _ => None,
        }
    }
    pub(crate) fn t(&self) -> Option<String> {
        match self {
            V::T(v) => v.clone(),
            _ => None,
        }
    }
}

pub(crate) fn t(s: impl Into<String>) -> V {
    V::T(Some(s.into()))
}
pub(crate) fn ot(s: Option<impl Into<String>>) -> V {
    V::T(s.map(Into::into))
}
pub(crate) fn i(v: i64) -> V {
    V::I(Some(v))
}

#[derive(Clone, Copy)]
pub(crate) enum Ty {
    I,
    F,
    T,
}

/// A team's test-replay tables in either backend. Statements use `$1`..
/// placeholders, each first appearing in order (SQLite numbers `$N`
/// parameters by first appearance).
pub(crate) enum Db {
    Sqlite(rusqlite::Connection),
    Pg(crate::pg::Pg),
}

impl Db {
    pub(crate) fn query(&self, sql: &str, params: &[V], tys: &[Ty]) -> anyhow::Result<Vec<Vec<V>>> {
        match self {
            Db::Sqlite(c) => {
                let mut st = c.prepare(sql)?;
                let ps: Vec<rusqlite::types::Value> = params
                    .iter()
                    .map(|v| match v {
                        V::I(x) => x.map_or(rusqlite::types::Value::Null, Into::into),
                        V::F(x) => x.map_or(rusqlite::types::Value::Null, Into::into),
                        V::T(x) => x.clone().map_or(rusqlite::types::Value::Null, Into::into),
                    })
                    .collect();
                let rows = st.query_map(rusqlite::params_from_iter(ps), |r| {
                    tys.iter()
                        .enumerate()
                        .map(|(k, ty)| {
                            Ok(match ty {
                                Ty::I => V::I(r.get(k)?),
                                Ty::F => V::F(r.get(k)?),
                                Ty::T => V::T(r.get(k)?),
                            })
                        })
                        .collect::<rusqlite::Result<Vec<V>>>()
                })?;
                Ok(rows.collect::<Result<_, _>>()?)
            }
            Db::Pg(pg) => {
                use r2d2_postgres::postgres::types::ToSql;
                let mut c = pg.conn()?;
                let owned: Vec<Box<dyn ToSql + Sync>> = params
                    .iter()
                    .map(|v| -> Box<dyn ToSql + Sync> {
                        match v {
                            V::I(x) => Box::new(*x),
                            V::F(x) => Box::new(*x),
                            V::T(x) => Box::new(x.clone()),
                        }
                    })
                    .collect();
                let refs: Vec<&(dyn ToSql + Sync)> = owned.iter().map(|b| b.as_ref()).collect();
                let rows = c.query(sql, &refs)?;
                rows.iter()
                    .map(|r| {
                        tys.iter()
                            .enumerate()
                            .map(|(k, ty)| {
                                Ok(match ty {
                                    Ty::I => V::I(r.try_get(k)?),
                                    Ty::F => V::F(r.try_get(k)?),
                                    Ty::T => V::T(r.try_get(k)?),
                                })
                            })
                            .collect::<anyhow::Result<Vec<V>>>()
                    })
                    .collect()
            }
        }
    }

    /// An INSERT ... RETURNING id.
    pub(crate) fn insert(&self, sql: &str, params: &[V]) -> anyhow::Result<i64> {
        self.query(sql, params, &[Ty::I])?
            .first()
            .and_then(|r| r[0].i())
            .ok_or_else(|| anyhow::anyhow!("no id returned"))
    }
}

fn db(app: &App, team: &str) -> Result<Db, ApiError> {
    db_with(app, team, "test-replay.sqlite", SCHEMA)
}

/// A team's tables in `file` (self-hosted: a SQLite file in the team's
/// directory, `schema` created in it) or in Postgres (Quod Cloud: the
/// tables are created at start, `boot::connect`).
pub(crate) fn db_with(app: &App, team: &str, file: &str, schema: &str) -> Result<Db, ApiError> {
    if let Some(c) = app.store.cloud_backend() {
        return Ok(Db::Pg(c.pg.clone()));
    }
    let p = app.store.team_dir(team).join(file);
    std::fs::create_dir_all(app.store.team_dir(team)).map_err(ApiError::internal)?;
    let c = rusqlite::Connection::open(&p).map_err(ApiError::internal)?;
    c.busy_timeout(std::time::Duration::from_secs(30))
        .map_err(ApiError::internal)?;
    c.pragma_update(None, "journal_mode", "WAL")
        .map_err(ApiError::internal)?;
    c.execute_batch(
        &schema
            .replace("{ID}", "INTEGER PRIMARY KEY AUTOINCREMENT")
            .replace("{REAL}", "REAL"),
    )
    .map_err(ApiError::internal)?;
    Ok(Db::Sqlite(c))
}

/// Where a runner's repository is keyed: GitHub's id when known (a
/// rename keeps the ledgers, a new repository under an old name starts
/// empty), else its name.
pub(crate) fn repo_key(r: &Runner) -> Result<String, ApiError> {
    if r.repository.is_empty() {
        return Err(ApiError::forbidden("this runner token names no repository"));
    }
    Ok(match r.repository_id {
        Some(id) => format!("github-{id}"),
        None => format!("name-{}", r.repository),
    })
}

/// Whether a job may save a ledger: not one on someone else's input.
pub(crate) fn may_save(r: &Runner) -> bool {
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

const COLS: &str = "id, at, scope, git_ref, sha, files";
const COL_TYS: &[Ty] = &[Ty::I, Ty::T, Ty::T, Ty::T, Ty::T, Ty::T];

fn row(v: &[V]) -> Row {
    Row {
        id: v[0].i().unwrap_or_default(),
        at: v[1].t().unwrap_or_default(),
        scope: v[2].t().unwrap_or_default(),
        git_ref: v[3].t(),
        sha: v[4].t(),
        files: v[5].t().unwrap_or_default(),
    }
}

/// The ledger a lookup answers with: the newest of the first ancestor
/// commit any read scope has (the job's own scope first), else the newest
/// of the job's own scope, else the newest of `trusted`. Two queries
/// whatever the number of ancestors: the readable rows' heads (newest
/// first, no file lists), then the chosen row.
fn find(
    db: &Db,
    team: &str,
    repo: &str,
    suite: &str,
    scopes: &[String],
    ancestors: &[String],
) -> anyhow::Result<Option<(Row, &'static str)>> {
    let own = scopes.first().cloned().unwrap_or_default();
    let last = scopes.last().cloned().unwrap_or_default();
    // (id, scope, sha), newest first.
    let heads: Vec<(i64, String, Option<String>)> = db
        .query(
            "SELECT id, scope, sha FROM test_ledgers WHERE team = $1 AND repo_key = $2 AND suite = $3 AND (scope = $4 OR scope = $5) ORDER BY id DESC LIMIT 5000",
            &[
                t(team),
                t(repo),
                t(suite),
                t(own.as_str()),
                t(last.as_str()),
            ],
            &[Ty::I, Ty::T, Ty::T],
        )?
        .into_iter()
        .map(|v| (v[0].i().unwrap_or_default(), v[1].t().unwrap_or_default(), v[2].t()))
        .collect();
    let mut pick: Option<(i64, &'static str)> = None;
    for sha in ancestors.iter().filter(|s| valid_sha(s)) {
        let at: Vec<&(i64, String, Option<String>)> = heads
            .iter()
            .filter(|h| h.2.as_deref() == Some(sha.as_str()))
            .collect();
        if let Some(h) = at.iter().find(|h| h.1 == own).or_else(|| at.first()) {
            pick = Some((h.0, "ancestor"));
            break;
        }
    }
    if pick.is_none() {
        for s in scopes {
            if let Some(h) = heads.iter().find(|h| h.1 == *s) {
                pick = Some((h.0, if s == TRUSTED { "default" } else { "branch" }));
                break;
            }
        }
    }
    let Some((id, how)) = pick else {
        return Ok(None);
    };
    let rows = db.query(
        &format!("SELECT {COLS} FROM test_ledgers WHERE team = $1 AND id = $2"),
        &[t(team), i(id)],
        COL_TYS,
    )?;
    Ok(rows.first().map(|v| (row(v), how)))
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
        let found = find(&d, &runner.team, &repo, &req.suite, &scopes, &req.ancestors)?;
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
                    repository_id: runner.repository_id,
                    requester_scope: runner.write.clone(),
                    ledger_id: r.id,
                    sha: r.sha.clone(),
                    files: files_hash(&files).map_err(ApiError::internal)?,
                    issued_at: now_rfc3339(),
                    nonce: req.nonce.clone(),
                };
                let receipt = sign_ledger(&app.settings.signing_key, statement)
                    .map_err(ApiError::internal)?;
                let bundle = d
                    .query(
                        "SELECT hash, size FROM test_bundles WHERE team = $1 AND ledger_id = $2",
                        &[t(runner.team.as_str()), i(r.id)],
                        &[Ty::T, Ty::I],
                    )?
                    .first()
                    .and_then(|v| {
                        Some(BundleRef {
                            hash: v[0].t()?.parse().ok()?,
                            size: v[1].i()? as u64,
                        })
                    });
                Some(FoundLedger {
                    bundle,
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
        // A delegated token states its own scope; the vitest client runs
        // inside the job, so that would let any job holding it write
        // `trusted`. Static runner tokens have a fixed scope (writer-only
        // tokens: see docs/test-replay-cloud.md).
        if matches!(principal, Principal::Delegated { .. }) {
            return Err(ApiError::forbidden(
                "a delegated runner token cannot save test ledgers; use the job's OIDC token",
            ));
        }
        let runner = App::runner_of(&principal, req.scope.as_ref())?;
        check_suite(&req.suite)?;
        if !may_save(&runner) {
            return Err(ApiError::forbidden(
                "this job runs on input someone else controls; it may read the default branch's test ledger but not save one",
            ));
        }
        check_files(&req.files).map_err(ApiError::bad_request)?;
        let repo = repo_key(&runner)?;
        let blobs = app.store.cas(&runner.team)?;
        // Every listed file must be on the server, with its listed size.
        let mut uniq: Vec<&LedgerFile> = req.files.iter().collect();
        uniq.sort_by_key(|f| f.hash);
        uniq.dedup_by_key(|f| f.hash);
        let sizes: Vec<(Hash, u64, Option<u64>)> = std::thread::scope(|s| {
            let chunks: Vec<_> = uniq
                .chunks(uniq.len().div_ceil(8).max(1))
                .map(|c| {
                    let blobs = &blobs;
                    s.spawn(move || {
                        c.iter()
                            .map(|f| Ok((f.hash, f.size, blobs.size(&f.hash)?)))
                            .collect::<Result<Vec<_>, quod_cas::CasError>>()
                    })
                })
                .collect();
            chunks
                .into_iter()
                .map(|h| h.join().expect("size check thread"))
                .collect::<Result<Vec<Vec<_>>, _>>()
                .map(|v| v.into_iter().flatten().collect())
        })
        .map_err(ApiError::internal)?;
        let mut missing = vec![];
        for (h, want, got) in sizes {
            match got {
                None => missing.push(h),
                Some(n) if n != want => {
                    return Err(ApiError::bad_request(format!(
                        "{h} is listed with another size than its blob"
                    )));
                }
                Some(_) => {}
            }
        }
        // One hash listed with two sizes (only the first was checked above).
        let mut size_of = std::collections::HashMap::new();
        for f in &req.files {
            if *size_of.entry(f.hash).or_insert(f.size) != f.size {
                return Err(ApiError::bad_request("one hash listed with two sizes"));
            }
        }
        if !missing.is_empty() {
            return Err(ApiError::missing_blobs(missing));
        }
        let c = ci(&runner, &req.ci);
        let mut files = req.files.clone();
        files.sort();
        let fh = files_hash(&files).map_err(ApiError::internal)?;
        let bytes: u64 = files.iter().map(|f| f.size).sum();
        let cap = |s: Option<&str>| s.map(crate::api::cap);
        let d = db(&app, &runner.team)?;
        let id = d.insert(
            "INSERT INTO test_ledgers (team, at, repo_key, repository, suite, scope, git_ref, sha, files, files_hash, n_files, bytes, run_id, workflow, job)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15) RETURNING id",
            &[
                t(runner.team.as_str()), t(now_rfc3339()), t(repo.as_str()), t(runner.repository.as_str()),
                t(req.suite.as_str()), t(runner.write.as_str()), ot(cap(c.git_ref.as_deref())), ot(sha_of(&c)),
                t(serde_json::to_string(&files).map_err(ApiError::internal)?), t(fh.hex()),
                i(files.len() as i64), i(bytes as i64),
                ot(cap(c.run_id.as_deref())), ot(cap(c.workflow.as_deref())), ot(cap(c.job.as_deref())),
            ],
        )?;
        tracing::info!(target: "quod", team = %runner.team, repository = %runner.repository, scope = %runner.write, suite = %req.suite, files = files.len(), bytes, "test ledger saved");
        // The whole ledger as one object, so a restore is one download.
        // Best effort: without it the client fetches file by file.
        if bytes <= MAX_BUNDLE {
            let built = build_bundle(&blobs, &files).and_then(|(h, n)| {
                d.query(
                    "INSERT INTO test_bundles (team, ledger_id, hash, size) VALUES ($1, $2, $3, $4) RETURNING ledger_id",
                    &[t(runner.team.as_str()), i(id), t(h.to_string()), i(n as i64)],
                    &[Ty::I],
                )
                .map(|_| n)
            });
            match built {
                Ok(n) => tracing::info!(target: "quod", ledger = id, bytes = n, "test ledger bundle stored"),
                Err(e) => tracing::warn!(target: "quod", ledger = id, error = %format!("{e:#}"), "test ledger bundle not stored"),
            }
        }
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
/// (`PackRequest`). Team-wide, as `GET /v1/blobs/{hash}` is. Up to 16
/// reads in flight (the bucket's latency, not its bandwidth, is the cost).
pub async fn pack(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<PackRequest>,
) -> Result<Response, ApiError> {
    let team = App::runner_team_of(&principal)?;
    if req.hashes.len() > MAX_PACK {
        return Err(ApiError::bad_request("too many hashes"));
    }
    let mut hashes = req.hashes;
    {
        let mut seen = std::collections::HashSet::new();
        hashes.retain(|h| seen.insert(*h));
    }
    // A few packs at a time per instance; each holds at most 32 blobs of at
    // most PACK_BLOB bytes in memory, and sends at most MAX_LEDGER_BYTES.
    let permit = PACKS.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "busy",
            "too many downloads at once; try again",
        )
    })?;
    let sent = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let blobs = {
        let a = app.clone();
        let t = team.clone();
        Arc::new(blocking(move || Ok(a.store.cas(&t)?)).await?)
    };
    let stream = futures_util::stream::iter(hashes)
        .map(move |h| {
            let blobs = blobs.clone();
            let sent = sent.clone();
            async move {
                let got = tokio::task::spawn_blocking(move || {
                    // Too large for a pack, or past the pack's total: "-"
                    // (the client then runs every test file).
                    // One request per blob: the bucket's answer says the
                    // length before the bytes are read.
                    let Some(b) = blobs.read_capped(&h, PACK_BLOB)? else {
                        return Ok(None);
                    };
                    let total = sent
                        .fetch_add(b.len() as u64, std::sync::atomic::Ordering::Relaxed)
                        + b.len() as u64;
                    Ok::<_, quod_cas::CasError>((total <= MAX_LEDGER_BYTES).then_some(b))
                })
                .await
                .map_err(std::io::Error::other)?;
                let out = match got {
                    Ok(Some(b)) => {
                        let mut o = format!("{} {}\n", h.hex(), b.len()).into_bytes();
                        o.extend_from_slice(&b);
                        o
                    }
                    Ok(None) | Err(_) => format!("{} -\n", h.hex()).into_bytes(),
                };
                Ok::<_, std::io::Error>(axum::body::Bytes::from(out))
            }
        })
        .buffered(32);
    // The permit lives as long as the body streams.
    let stream = stream.map(move |x| {
        let _held = &permit;
        x
    });
    Ok((
        [("content-type", "application/octet-stream")],
        Body::from_stream(stream),
    )
        .into_response())
}

/// The largest ledger stored as one bundle object.
const MAX_BUNDLE: u64 = 64 << 20;

/// Reads a ledger's blobs (32 at a time) and stores them as one object in
/// pack format, each blob once, in hash order. Returns its hash and size.
fn build_bundle(
    blobs: &crate::store::TeamBlobs,
    files: &[LedgerFile],
) -> anyhow::Result<(Hash, u64)> {
    let mut hashes: Vec<Hash> = files.iter().map(|f| f.hash).collect();
    hashes.sort();
    hashes.dedup();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let got: std::sync::Mutex<Vec<Option<Vec<u8>>>> =
        std::sync::Mutex::new(vec![None; hashes.len()]);
    let failed: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    std::thread::scope(|sc| {
        for _ in 0..32.min(hashes.len()) {
            sc.spawn(|| {
                loop {
                    let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(h) = hashes.get(k) else { break };
                    match blobs.read_capped(h, MAX_BUNDLE) {
                        Ok(Some(b)) => got.lock().unwrap_or_else(|e| e.into_inner())[k] = Some(b),
                        Ok(None) => {
                            *failed.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(format!("{h} is too large"));
                            break;
                        }
                        Err(e) => {
                            *failed.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(format!("{h}: {e}"));
                            break;
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = failed.into_inner().unwrap_or_else(|e| e.into_inner()) {
        anyhow::bail!(e);
    }
    let mut out = Vec::new();
    for (h, b) in hashes
        .iter()
        .zip(got.into_inner().unwrap_or_else(|e| e.into_inner()))
    {
        let b = b.ok_or_else(|| anyhow::anyhow!("{h} not read"))?;
        out.extend_from_slice(format!("{} {}\n", h.hex(), b.len()).as_bytes());
        out.extend_from_slice(&b);
    }
    let n = out.len() as u64;
    let h = blobs.put_blob(&out)?;
    Ok((h, n))
}

/// Packs streaming at once per instance.
static PACKS: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));
/// The largest blob a pack carries.
const PACK_BLOB: u64 = 16 << 20;
/// The most a report's stored detail may be, in bytes.
const MAX_DETAIL: usize = 1 << 20;

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
        // Reports are numbers people trust (spot-checks, wrong results):
        // only jobs whose ledgers count may send them.
        if !may_save(&runner) {
            return Err(ApiError::forbidden(
                "this job runs on input someone else controls; it does not report test runs",
            ));
        }
        let r = &req.report;
        if !matches!(r.kind.as_str(), "critical" | "shadow") || r.tool != "vitest" {
            return Err(ApiError::bad_request(
                "kind is critical or shadow; tool is vitest",
            ));
        }
        if r.ran_files.len() > MAX_REPORT_FILES
            || r.audit_files.len() > MAX_REPORT_FILES
            || r.record_differs.len() > MAX_REPORT_FILES
            || r.replayed_files.len() > MAX_FILES
        {
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
        let mut ran_files: Vec<RanFile> = r
            .ran_files
            .iter()
            .map(|f| RanFile {
                file: capped(&f.file, 1024),
                project: capped(&f.project, 200),
                why: capped(&f.why, 1024),
                duration_ms: finite(f.duration_ms),
                state: capped(&f.state, 40),
                unrecordable: f.unrecordable.as_deref().map(|s| capped(s, 300)),
            })
            .collect();
        let record_differs: Vec<String> =
            r.record_differs.iter().map(|f| capped(f, 1024)).collect();
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
        let mut detail = json!({ "ranFiles": ran_files, "auditFiles": audit_files, "ledger": ledger, "recordDiffers": record_differs });
        while detail.to_string().len() > MAX_DETAIL && !ran_files.is_empty() {
            let keep = ran_files.len() / 2;
            ran_files.truncate(keep);
            detail = json!({ "ranFiles": ran_files, "auditFiles": audit_files, "ledger": ledger, "recordDiffers": record_differs, "ranFilesTruncated": true });
        }
        if detail.to_string().len() > MAX_DETAIL {
            return Err(ApiError::bad_request("the report is too large"));
        }
        let cap = |s: &Option<String>| ot(s.as_deref().map(crate::api::cap));
        let d = db(&app, &runner.team)?;
        let id = d.insert(
            "INSERT INTO test_runs (team, at, repo_key, repository, suite, scope, kind, tool, run_id, run_attempt, run_number, workflow, job, job_name, git_ref, sha, event, actor,
               total, ran, replayed, narrowed, wall_ms, select_ms, est_plain_ms, saved_ms, exit, audited, audit_mismatches, detail)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$30) RETURNING id",
            &[
                t(runner.team.as_str()), t(now_rfc3339()), t(repo.as_str()), t(runner.repository.as_str()),
                t(req.suite.as_str()), t(runner.write.as_str()), t(r.kind.as_str()), t(r.tool.as_str()),
                cap(&c.run_id), V::I(c.run_attempt.map(|v| v as i64)), V::I(c.run_number.map(|v| v as i64)),
                cap(&c.workflow), cap(&c.job), cap(&c.job_name), cap(&c.git_ref), ot(sha_of(&c)), cap(&c.event), cap(&c.actor),
                i(r.total as i64), i(r.ran as i64), i(r.replayed as i64), i(r.narrowed as i64),
                V::F(finite(r.wall_ms)), V::F(finite(r.select_ms)), V::F(finite(r.est_plain_ms)), V::F(finite(r.saved_ms)),
                V::I(r.exit.map(i64::from)), i(r.audited as i64), i(mismatches), t(detail.to_string()),
            ],
        )?;
        // Per-file stats: a check's skips (and the recorded time they
        // saved), a record run's reasons a file always runs. Best effort.
        if let Err(e) = file_stats(&d, &runner.team, &repo, &req.suite, r, &ran_files, &record_differs) {
            tracing::warn!(target: "quod", error = %format!("{e:#}"), "test file stats not written");
        }
        // The live event: the summary as the dashboard lists it.
        let out = (|| -> anyhow::Result<()> {
            let repos = all_repos(&app, &runner.team, &d)?;
            let run = load_run(&d, &runner.team, id)?.ok_or_else(|| anyhow::anyhow!("row gone"))?;
            let summary = summary(&d, &runner.team, &run, &repos)?;
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

/// One stats row to write: (file, skipped, saved_ms, reason, reason_kind,
/// whether it sets the reason).
type StatRow = (String, i64, f64, Option<String>, Option<String>, bool);

/// Updates `test_file_stats` from a report: every file a check replayed
/// counts one skip and its recorded time; a record run sets each file's
/// reason it always runs (unrecordable, or its recording differed from the
/// check), and clears it for a file it recorded.
fn file_stats(
    d: &Db,
    team: &str,
    repo: &str,
    suite: &str,
    r: &TestReport,
    ran: &[RanFile],
    differs: &[String],
) -> anyhow::Result<()> {
    let now = now_rfc3339();
    // (file, skipped, saved_ms, reason, reason_kind, set_reason)
    let mut rows: Vec<StatRow> = vec![];
    if r.kind == "critical" {
        for f in &r.replayed_files {
            rows.push((
                capped(&f.file, 1024),
                1,
                finite(f.duration_ms).unwrap_or(0.0),
                None,
                None,
                false,
            ));
        }
    } else {
        for f in ran {
            if differs.contains(&f.file) {
                continue;
            }
            match &f.unrecordable {
                Some(why) => rows.push((
                    f.file.clone(),
                    0,
                    0.0,
                    Some(why.clone()),
                    Some("unrecordable".into()),
                    true,
                )),
                None => rows.push((f.file.clone(), 0, 0.0, None, None, true)),
            }
        }
        for f in differs {
            rows.push((
                f.clone(),
                0,
                0.0,
                Some("its recorded run behaved differently from the check".into()),
                Some("differs".into()),
                true,
            ));
        }
    }
    for set_reason in [false, true] {
        let mine: Vec<&StatRow> = rows.iter().filter(|x| x.5 == set_reason).collect();
        for chunk in mine.chunks(400) {
            let mut params = vec![t(team), t(repo), t(suite), t(now.as_str())];
            let mut values = vec![];
            for (k, (file, skipped, saved, reason, kind, _)) in chunk.iter().enumerate() {
                let b = 5 + k * 5;
                values.push(format!(
                    "($1, $2, $3, $4, ${}, ${}, ${}, ${}, ${})",
                    b,
                    b + 1,
                    b + 2,
                    b + 3,
                    b + 4
                ));
                params.extend([
                    t(file.as_str()),
                    i(*skipped),
                    V::F(Some(*saved)),
                    ot(reason.clone()),
                    ot(kind.clone()),
                ]);
            }
            let update = if set_reason {
                "reason = excluded.reason, reason_kind = excluded.reason_kind, last_at = excluded.last_at"
            } else {
                "skipped = test_file_stats.skipped + excluded.skipped, saved_ms = test_file_stats.saved_ms + excluded.saved_ms, last_at = excluded.last_at"
            };
            d.query(
                &format!(
                    "INSERT INTO test_file_stats (team, repo_key, suite, last_at, file, skipped, saved_ms, reason, reason_kind) VALUES {} ON CONFLICT (team, repo_key, suite, file) DO UPDATE SET {update} RETURNING skipped",
                    values.join(", ")
                ),
                &params,
                &[Ty::I],
            )?;
        }
    }
    Ok(())
}

/// A suite's per-file stats: file -> (skipped, saved_ms, reason, kind).
type FileStats = std::collections::HashMap<String, (i64, f64, Option<String>, Option<String>)>;

fn suite_stats(d: &Db, team: &str) -> anyhow::Result<BTreeMap<(String, String), FileStats>> {
    let mut out: BTreeMap<(String, String), FileStats> = BTreeMap::new();
    for r in d.query(
        "SELECT repo_key, suite, file, skipped, saved_ms, reason, reason_kind FROM test_file_stats WHERE team = $1",
        &[t(team)],
        &[Ty::T, Ty::T, Ty::T, Ty::I, Ty::F, Ty::T, Ty::T],
    )? {
        if let (Some(rk), Some(s), Some(f)) = (r[0].t(), r[1].t(), r[2].t()) {
            out.entry((rk, s)).or_default().insert(
                f,
                (r[3].i().unwrap_or(0), r[4].f().unwrap_or(0.0), r[5].t(), r[6].t()),
            );
        }
    }
    Ok(out)
}

/// An old suite name the client used before suites were named by their
/// directory (`apps_web-4f53cda1`): retired when the repository has the
/// new name (`apps/web`), so a renamed suite counts once.
fn retired(suite: &str, others: &BTreeSet<&str>) -> bool {
    let Some((base, tail)) = suite.rsplit_once('-') else {
        return false;
    };
    let new = base.replace('_', "/");
    tail.len() == 8
        && tail.bytes().all(|b| b.is_ascii_hexdigit())
        && (others.contains(new.as_str()) || others.contains(base))
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
const RUN_TYS: &[Ty] = &[
    Ty::I,
    Ty::T,
    Ty::T,
    Ty::T,
    Ty::T,
    Ty::T,
    Ty::T,
    Ty::T,
    Ty::T,
    Ty::I,
    Ty::I,
    Ty::T,
    Ty::T,
    Ty::T,
    Ty::T,
    Ty::T,
    Ty::T,
    Ty::I,
    Ty::I,
    Ty::I,
    Ty::I,
    Ty::F,
    Ty::F,
    Ty::F,
    Ty::F,
    Ty::I,
    Ty::I,
    Ty::I,
    Ty::T,
];

fn run_row(v: &[V]) -> RunRow {
    let s = |k: usize| v[k].t().unwrap_or_default();
    let n = |k: usize| v[k].i().unwrap_or_default();
    RunRow {
        id: n(0),
        at: s(1),
        repo_key: s(2),
        repository: s(3),
        suite: s(4),
        scope: s(5),
        kind: s(6),
        tool: s(7),
        run_id: v[8].t(),
        run_attempt: v[9].i(),
        run_number: v[10].i(),
        workflow: v[11].t(),
        job: v[12].t(),
        job_name: v[13].t(),
        git_ref: v[14].t(),
        sha: v[15].t(),
        event: v[16].t(),
        total: n(17),
        ran: n(18),
        replayed: n(19),
        narrowed: n(20),
        wall_ms: v[21].f(),
        select_ms: v[22].f(),
        est_plain_ms: v[23].f(),
        saved_ms: v[24].f(),
        exit: v[25].i(),
        audited: n(26),
        audit_mismatches: n(27),
        detail: s(28),
    }
}

fn runs_where(d: &Db, cond: &str, params: &[V]) -> anyhow::Result<Vec<RunRow>> {
    Ok(d.query(
        &format!("SELECT {RUN_COLS} FROM test_runs WHERE {cond}"),
        params,
        RUN_TYS,
    )?
    .iter()
    .map(|v| run_row(v))
    .collect())
}

fn load_run(d: &Db, team: &str, id: i64) -> anyhow::Result<Option<RunRow>> {
    Ok(runs_where(d, "team = $1 AND id = $2", &[t(team), i(id)])?
        .into_iter()
        .next())
}

/// The shadow report of the same commit, suite, scope and workflow run
/// as a critical one.
fn shadow_of(d: &Db, team: &str, r: &RunRow) -> anyhow::Result<Option<RunRow>> {
    let Some(sha) = &r.sha else { return Ok(None) };
    // The same workflow run too, when known: two runs of one commit (a
    // re-dispatch, a benchmark) each show their own shadow's checks.
    Ok(runs_where(
        d,
        "team = $1 AND repo_key = $2 AND suite = $3 AND sha = $4 AND scope = $5 AND kind = 'shadow' AND ($6 = '' OR run_id = $6) ORDER BY id DESC LIMIT 1",
        &[
            t(team),
            t(r.repo_key.as_str()),
            t(r.suite.as_str()),
            t(sha.as_str()),
            t(r.scope.as_str()),
            t(r.run_id.clone().unwrap_or_default()),
        ],
    )?
    .into_iter()
    .next())
}

/// Every repository name the team has (step scopes and test runs), for
/// dashboard slugs.
fn all_repos(app: &App, team: &str, d: &Db) -> anyhow::Result<BTreeSet<String>> {
    let mut s: BTreeSet<String> = app
        .store
        .scopes(team)?
        .into_iter()
        .map(|s| s.repository)
        .collect();
    for r in d.query(
        "SELECT DISTINCT repository FROM test_runs WHERE team = $1",
        &[t(team)],
        &[Ty::T],
    )? {
        if let Some(x) = r[0].t() {
            s.insert(x);
        }
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
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
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
fn summary(d: &Db, team: &str, r: &RunRow, repos: &BTreeSet<String>) -> anyhow::Result<Value> {
    let mut v = base_summary(r, repos);
    if r.kind == "critical"
        && r.replayed > 0
        && let Some(s) = shadow_of(d, team, r)?
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

/// The team's newest runs (one query), newest first.
fn team_runs(d: &Db, team: &str) -> anyhow::Result<Vec<RunRow>> {
    runs_where(d, "team = $1 ORDER BY id DESC LIMIT 5000", &[t(team)])
}

/// Repository names for slugs: the step scopes' and these runs', with no
/// second query over the runs.
fn repos_of(app: &App, team: &str, rows: &[RunRow]) -> anyhow::Result<BTreeSet<String>> {
    let mut s: BTreeSet<String> = app
        .store
        .scopes(team)?
        .into_iter()
        .map(|s| s.repository)
        .collect();
    s.extend(rows.iter().map(|r| r.repository.clone()));
    Ok(s)
}

/// The shadows among `rows` (newest first), by commit, suite, scope and
/// workflow run (and by commit, suite and scope alone, for a critical run
/// that names no run): `shadow_of` in memory.
struct Shadows<'a> {
    by_run: std::collections::HashMap<(&'a str, &'a str, &'a str, &'a str, &'a str), &'a RunRow>,
    by_commit: std::collections::HashMap<(&'a str, &'a str, &'a str, &'a str), &'a RunRow>,
}

impl<'a> Shadows<'a> {
    fn of(rows: &'a [RunRow]) -> Shadows<'a> {
        let mut by_run = std::collections::HashMap::new();
        let mut by_commit = std::collections::HashMap::new();
        for r in rows.iter().filter(|r| r.kind == "shadow") {
            let Some(sha) = r.sha.as_deref() else {
                continue;
            };
            let k = (r.repo_key.as_str(), r.suite.as_str(), sha, r.scope.as_str());
            by_commit.entry(k).or_insert(r);
            if let Some(id) = r.run_id.as_deref() {
                by_run.entry((k.0, k.1, k.2, k.3, id)).or_insert(r);
            }
        }
        Shadows { by_run, by_commit }
    }

    fn get(&self, r: &RunRow) -> Option<&'a RunRow> {
        if r.kind != "critical" {
            return None;
        }
        let sha = r.sha.as_deref()?;
        let k = (r.repo_key.as_str(), r.suite.as_str(), sha, r.scope.as_str());
        match r.run_id.as_deref().filter(|s| !s.is_empty()) {
            Some(id) => self.by_run.get(&(k.0, k.1, k.2, k.3, id)).copied(),
            None => self.by_commit.get(&k).copied(),
        }
    }

    fn summary(&self, r: &RunRow, repos: &BTreeSet<String>) -> Value {
        let mut v = base_summary(r, repos);
        if r.replayed > 0
            && let Some(s) = self.get(r)
        {
            v["audits"] = json!({"checked": s.audited, "mismatched": s.audit_mismatches});
        }
        v
    }
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
        let rows = team_runs(&d, &v.team)?;
        let repos = repos_of(&app, &v.team, &rows)?;
        let shadows = Shadows::of(&rows);
        let limit = q.limit.unwrap_or(50).clamp(1, 200);
        let out: Vec<Value> = rows
            .iter()
            .filter(|r| {
                q.repo
                    .as_ref()
                    .is_none_or(|s| crate::dash::slug(&r.repository, &repos) == *s)
            })
            .take(limit)
            .map(|r| shadows.summary(r, &repos))
            .collect();
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
        let r = load_run(&d, &v.team, id)?.ok_or_else(|| ApiError::not_found("no such run"))?;
        let repos = all_repos(&app, &v.team, &d)?;
        let mut out = summary(&d, &v.team, &r, &repos)?;
        let detail: Value = serde_json::from_str(&r.detail).unwrap_or(json!({}));
        let shadow = if r.kind == "critical" {
            shadow_of(&d, &v.team, &r)?
        } else {
            None
        };
        let audit_source = shadow.as_ref().unwrap_or(&r);
        let audit_detail: Value = serde_json::from_str(&audit_source.detail).unwrap_or(json!({}));
        // The dashboard's names (the stored detail keeps the wire's).
        out["ranFiles"] = Value::Array(
            detail["ranFiles"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| {
                    json!({
                        "file": f["file"], "project": f["project"], "why": f["why"],
                        "durationMs": f["duration_ms"], "state": f["state"],
                    })
                })
                .collect(),
        );
        // Spot-checks apply to a check only for files it reused (not ones
        // it ran), and not at all when it reused nothing.
        let ran_here: BTreeSet<&str> = detail["ranFiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f["file"].as_str())
            .collect();
        out["auditFiles"] = if r.kind == "critical" && r.replayed == 0 {
            json!([])
        } else {
            Value::Array(
                audit_detail["auditFiles"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|a| {
                        r.kind != "critical"
                            || a["file"].as_str().is_some_and(|f| !ran_here.contains(f))
                    })
                    .cloned()
                    .collect(),
            )
        };
        if r.kind == "critical" && out["auditFiles"].as_array().is_some_and(|a| a.is_empty()) {
            out["audits"] = Value::Null;
        }
        // Why a file ran: what the record run found, when the check could
        // only say it was not recorded.
        let stats = suite_stats(&d, &v.team)?;
        if let Some(st) = stats.get(&(r.repo_key.clone(), r.suite.clone()))
            && let Some(files) = out["ranFiles"].as_array_mut()
        {
            for f in files {
                let why = f["why"].as_str().unwrap_or("");
                if !(why.starts_with("new test file") || why.starts_with("first run")) {
                    continue;
                }
                if let Some((_, _, Some(reason), Some(kind))) =
                    f["file"].as_str().and_then(|n| st.get(n))
                {
                    f["why"] = json!(if kind == "differs" {
                        format!("always runs: recording differed ({reason})")
                    } else {
                        format!("always runs: {reason}")
                    });
                }
            }
        }
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
        // One query: the team's newest runs; the sums are over those (the
        // last 5000, far more than a week of any team today).
        let rows = team_runs(&d, &v.team)?;
        let repos = repos_of(&app, &v.team, &rows)?;
        let shadows = Shadows::of(&rows);
        let sums = |from: &str| {
            let c: Vec<&RunRow> = rows
                .iter()
                .filter(|r| r.kind == "critical" && r.at.as_str() >= from)
                .collect();
            (
                c.iter().filter_map(|r| r.saved_ms).sum::<f64>(),
                c.len() as i64,
                c.iter().map(|r| r.total).sum::<i64>(),
                c.iter().map(|r| r.replayed).sum::<i64>(),
            )
        };
        let (t_saved, t_runs, _, _) = sums(&today);
        let (w_saved, w_runs, w_total, w_skipped) = sums(&week);
        let (checked, mismatched) = rows
            .iter()
            .filter(|r| r.kind == "shadow")
            .fold((0i64, 0i64), |(c, m), r| (c + r.audited, m + r.audit_mismatches));
        let recent: Vec<Value> = rows
            .iter()
            .filter(|r| r.kind == "critical")
            .take(10)
            .map(|r| shadows.summary(r, &repos))
            .collect();
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

// ----- test files per suite (the Trust page) -----

/// A recording's per-file records (path, record), parsed from its
/// ledger.json in the bucket once per ledger version: a ledger row never
/// changes, so the id is the cache key.
type Recs = Arc<Vec<(String, Value)>>;
static RECS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(String, i64), Recs>>> =
    std::sync::LazyLock::new(Default::default);

fn ledger_recs(app: &App, team: &str, id: i64, files_json: &str) -> anyhow::Result<Recs> {
    let key = (team.to_string(), id);
    if let Some(r) = RECS.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(r.clone());
    }
    let files: Vec<LedgerFile> = serde_json::from_str(files_json)?;
    if files.is_empty() {
        // Not loaded (nothing to cache under this id).
        return Ok(Arc::new(vec![]));
    }
    let mut recs = vec![];
    if let Some(lf) = files.iter().find(|f| f.path == "ledger.json")
        && let Some(b) = app.store.cas(team)?.read_capped(&lf.hash, MAX_BUNDLE)?
    {
        let l: Value = serde_json::from_slice(&b)?;
        for rec in l["files"].as_object().into_iter().flat_map(|m| m.values()) {
            if let Some(f) = rec["file"].as_str() {
                recs.push((
                    f.to_string(),
                    json!({
                        "project": rec["project"], "state": rec["state"], "duration": rec["duration"],
                        "quarantined": rec["quarantined"], "runs": rec["runs"], "replays": rec["replays"],
                    }),
                ));
            }
        }
    }
    let recs = Arc::new(recs);
    let mut c = RECS.lock().unwrap_or_else(|e| e.into_inner());
    if c.len() > 256 {
        c.clear();
    }
    c.insert(key, recs.clone());
    Ok(recs)
}

/// The newest ledger head of a suite: the trusted scope's (every writer
/// branch saves there: main, dev, ...) first, else any scope's.
struct Head {
    id: i64,
    at: String,
    scope: String,
    git_ref: Option<String>,
    sha: Option<String>,
}

/// One suite's test files and their states, from its newest recording and
/// the runs: a file the recording replays is `ready` (never skipped yet)
/// or `skipping`; one a spot-check found different is `paused`; one
/// quarantined otherwise, or that ran but could not be recorded, `always`;
/// one the runs name that no recording has yet, `learning`.
fn suite_files(
    head: Option<&Head>,
    recs: &[(String, Value)],
    runs: &[&RunRow],
    stats: Option<&FileStats>,
) -> (Value, Vec<Value>) {
    let none = FileStats::default();
    let stats = stats.unwrap_or(&none);
    // Spot-checks that differed, and the newest two runs' files that ran.
    let details: Vec<(usize, Value)> = runs
        .iter()
        .enumerate()
        .filter(|(i, r)| *i < 2 || (r.kind == "shadow" && r.audit_mismatches > 0))
        .map(|(i, r)| (i, serde_json::from_str(&r.detail).unwrap_or(Value::Null)))
        .collect();
    let mut differed: BTreeSet<&str> = BTreeSet::new();
    let mut ran_why: BTreeMap<String, String> = BTreeMap::new();
    for (i, det) in &details {
        let r = runs[*i];
        if r.kind == "shadow" && r.audit_mismatches > 0 {
            for a in det["auditFiles"].as_array().into_iter().flatten() {
                if a["ok"] == false
                    && let Some(f) = a["file"].as_str()
                {
                    differed.insert(f);
                }
            }
        }
        if *i < 2 {
            for f in det["ranFiles"].as_array().into_iter().flatten() {
                if let (Some(file), Some(why)) = (f["file"].as_str(), f["why"].as_str()) {
                    ran_why
                        .entry(file.to_string())
                        .or_insert_with(|| why.to_string());
                }
            }
        }
    }
    let recorded: BTreeSet<&str> = recs.iter().map(|(f, _)| f.as_str()).collect();
    let mut out: Vec<Value> = vec![];
    for (file, rec) in recs {
        let st = stats.get(file);
        // Skips: the checks' count (every check reports what it reused),
        // not only the recordings'.
        let skipped = st
            .map_or(0, |s| s.0.max(0) as u64)
            .max(rec["replays"].as_u64().unwrap_or(0));
        let ran = rec["runs"].as_u64().unwrap_or(0);
        let dur = rec["duration"].as_f64();
        let last = rec["state"].as_str().map(str::to_string);
        let mut differs = false;
        let (state, reason) = if rec["quarantined"].as_bool() == Some(true) {
            if differed.contains(file.as_str()) {
                (
                    "paused",
                    Some("a spot-check found its result differed".to_string()),
                )
            } else {
                differs = true;
                (
                    "always",
                    Some(
                        "recording differed: its recorded run behaved differently from the check"
                            .to_string(),
                    ),
                )
            }
        } else if !matches!(last.as_deref(), Some("pass" | "skip")) {
            (
                "always",
                Some(format!(
                    "its last result was {}",
                    last.as_deref().unwrap_or("unknown")
                )),
            )
        } else if skipped > 0 {
            ("skipping", None)
        } else {
            ("ready", None)
        };
        let saved = match st {
            Some(s) if s.1 > 0.0 => Some(s.1),
            _ => dur.map(|d| d * skipped as f64),
        };
        out.push(json!({
            "file": file, "project": rec["project"].as_str().unwrap_or(""),
            "state": state, "reason": reason, "lastResult": last,
            "skipped": skipped, "ran": ran, "durationMs": dur,
            "savedMs": saved, "recordDiffers": differs,
        }));
    }
    for (file, why) in &ran_why {
        if recorded.contains(file.as_str()) {
            continue;
        }
        // What the record run found, when it said: the file cannot be
        // recorded (and why), or its recording differed from the check.
        let known = stats
            .get(file)
            .and_then(|s| Some((s.2.clone()?, s.3.clone()?)));
        let (state, reason, differs) = match known {
            Some((r, kind)) if kind == "differs" => {
                ("always", format!("recording differed: {r}"), true)
            }
            Some((r, _)) => ("always", r, false),
            None if head.is_none()
                || why.starts_with("first run")
                || why.starts_with("new test file") =>
            {
                ("learning", "not recorded yet".to_string(), false)
            }
            None => (
                "always",
                "not recorded: it failed, or did something the recorder cannot follow (a child process, the network, a worker)".to_string(),
                false,
            ),
        };
        out.push(json!({
            "file": file, "project": "",
            "state": state, "reason": reason,
            "lastResult": Value::Null, "skipped": 0, "ran": 0, "durationMs": Value::Null, "savedMs": Value::Null,
            "recordDiffers": differs,
        }));
    }
    let rank = |s: &str| match s {
        "paused" => 0,
        "always" => 1,
        "learning" => 2,
        "skipping" => 3,
        _ => 4,
    };
    out.sort_by(|a, b| {
        (rank(a["state"].as_str().unwrap_or("")), a["file"].as_str())
            .cmp(&(rank(b["state"].as_str().unwrap_or("")), b["file"].as_str()))
    });
    let ledger = match head {
        Some(h) => {
            json!({"at": h.at, "sha": h.sha, "branch": branch(h.git_ref.as_deref()), "scope": h.scope})
        }
        None => Value::Null,
    };
    (ledger, out)
}

pub async fn suites(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(team): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    blocking(move || {
        let v = crate::dash::viewer(&app, &headers, &team)?;
        let d = db(&app, &v.team)?;
        // Three queries for the page: the runs, the ledger heads, and the
        // chosen ledgers' file lists; each ledger's records come from the
        // bucket once per ledger version (`ledger_recs`).
        let rows = team_runs(&d, &v.team)?;
        let repos = repos_of(&app, &v.team, &rows)?;
        let mut keys: BTreeMap<(String, String), String> = BTreeMap::new();
        for r in rows.iter().rev() {
            keys.insert((r.repo_key.clone(), r.suite.clone()), r.repository.clone());
        }
        let mut heads: BTreeMap<(String, String), Head> = BTreeMap::new();
        for h in d.query(
            "SELECT id, repo_key, suite, scope, at, sha, git_ref, repository FROM test_ledgers WHERE team = $1 ORDER BY id DESC LIMIT 5000",
            &[t(v.team.as_str())],
            &[Ty::I, Ty::T, Ty::T, Ty::T, Ty::T, Ty::T, Ty::T, Ty::T],
        )? {
            let (Some(id), Some(rk), Some(suite), Some(scope)) = (h[0].i(), h[1].t(), h[2].t(), h[3].t()) else {
                continue;
            };
            let key = (rk, suite);
            if let Some(name) = h[7].t() {
                keys.entry(key.clone()).or_insert(name);
            }
            let better = match heads.get(&key) {
                None => true,
                // Newest first: replace only a non-trusted head by a trusted one.
                Some(cur) => cur.scope != TRUSTED && scope == TRUSTED,
            };
            if better {
                heads.insert(
                    key,
                    Head {
                        id,
                        at: h[4].t().unwrap_or_default(),
                        scope,
                        sha: h[5].t(),
                        git_ref: h[6].t(),
                    },
                );
            }
        }
        // The chosen ledgers' file lists, for those not cached yet.
        let cached = |id: i64| {
            RECS.lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&(v.team.clone(), id))
        };
        let need: Vec<i64> = heads.values().map(|h| h.id).filter(|id| !cached(*id)).collect();
        let mut files_of: std::collections::HashMap<i64, String> = std::collections::HashMap::new();
        for chunk in need.chunks(200) {
            let ors: Vec<String> = (0..chunk.len()).map(|k| format!("id = ${}", k + 2)).collect();
            let mut params = vec![t(v.team.as_str())];
            params.extend(chunk.iter().map(|id| i(*id)));
            for r in d.query(
                &format!("SELECT id, files FROM test_ledgers WHERE team = $1 AND ({})", ors.join(" OR ")),
                &params,
                &[Ty::I, Ty::T],
            )? {
                if let (Some(id), Some(f)) = (r[0].i(), r[1].t()) {
                    files_of.insert(id, f);
                }
            }
        }
        let stats = suite_stats(&d, &v.team)?;
        let mut by_repo: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (rk, s) in keys.keys() {
            by_repo.entry(rk.as_str()).or_default().insert(s.as_str());
        }
        let mut out = vec![];
        for ((repo_key, suite), repository) in &keys {
            if by_repo
                .get(repo_key.as_str())
                .is_some_and(|others| retired(suite, others))
            {
                continue;
            }
            let key = (repo_key.clone(), suite.clone());
            let head = heads.get(&key);
            let recs = match head {
                Some(h) => ledger_recs(
                    &app,
                    &v.team,
                    h.id,
                    files_of.get(&h.id).map(String::as_str).unwrap_or("[]"),
                )?,
                None => Arc::new(vec![]),
            };
            let runs: Vec<&RunRow> = rows
                .iter()
                .filter(|r| r.repo_key == *repo_key && r.suite == *suite)
                .collect();
            let (ledger, files) = suite_files(head, &recs, &runs, stats.get(&key));
            let count = |s: &str| files.iter().filter(|f| f["state"] == s).count();
            let saved: f64 = files.iter().filter_map(|f| f["savedMs"].as_f64()).sum();
            out.push(json!({
                "repoSlug": crate::dash::slug(repository, &repos),
                "repository": repository,
                "suite": suite,
                "ledger": ledger,
                "totals": {
                    "files": files.len(), "learning": count("learning"), "ready": count("ready"),
                    "skipping": count("skipping"), "always": count("always"), "paused": count("paused"),
                    "recordDiffers": files.iter().filter(|f| f["recordDiffers"] == true).count(),
                    "savedMs": saved,
                },
                "files": files,
            }));
        }
        Ok(Json(json!({ "suites": out })))
    })
    .await
}
