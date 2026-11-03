//! Task cache: turbo's Remote Cache API (`/v8/artifacts/...`) and Nx's
//! self-hosted remote cache API (`/nx/v1/cache/...`), so a monorepo's task
//! runner replays a task (a typecheck, a build) another job of the same
//! repository already ran. The task runner decides what is the same task
//! (its own hash); the server decides who may read and write what.
//! docs/task-cache.md has the design.
//!
//! Auth. The task runner sends a fixed bearer token for the whole job
//! (`TURBO_TOKEN`, `NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN`), but a GitHub
//! OIDC token lives five minutes. So the action trades the job's OIDC token
//! for a session (`POST /v1/task-cache/session`): the runner it proved
//! (team, repository, scopes) and the job's environment, MACed with a key
//! derived from the server's signing key, valid for six hours.
//!
//! Trust is test replay's (`testreplay`'s module docs): a writer job
//! (push, dispatch or schedule on a writer ref) writes and reads `trusted`;
//! the repository's other jobs (another branch, a pull request) write their
//! ref's own scope and read it, then `trusted`; jobs on someone else's
//! input and fork scopes read `trusted` and never write. So a pull
//! request's artifact is never what the default branch reads.
//!
//! An artifact is keyed by (team, repository, environment, tool, scope,
//! the tool's hash). The environment (`ns`: the runner's OS, architecture,
//! node version and image, as the action states them) is not in turbo's or
//! Nx's hash: a node 20 job never replays a node 22 job's test task. Rows
//! are only ever added: the first artifact for a key stays (turbo's hash
//! names the task's inputs, so a second upload is the same task's output).
//! The bytes go into the team's blobs (the bucket in Quod Cloud).

use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use quod_cas::{BlobStore, Hash};
use quod_ledger::now_rfc3339;
use serde::{Deserialize, Serialize};

use crate::auth::Principal;
use crate::store::TeamBlobs;
use crate::testreplay::{Db, Ty, V, db_with, i, may_save, ot, read_scopes, repo_key, t};
use crate::{ApiError, App, AppState, blocking};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS task_cache (
  team TEXT NOT NULL, repo_key TEXT NOT NULL, ns TEXT NOT NULL, tool TEXT NOT NULL,
  scope TEXT NOT NULL, hash TEXT NOT NULL, blob TEXT NOT NULL, size BIGINT NOT NULL,
  duration BIGINT, tag TEXT, at TEXT NOT NULL, repository TEXT NOT NULL,
  PRIMARY KEY (team, repo_key, ns, tool, scope, hash));
";

/// Creates the table in Postgres (at start, from `boot::connect`).
pub fn ensure_pg(pg: &crate::pg::Pg) -> anyhow::Result<()> {
    let sql = SCHEMA.replace("TEXT", "TEXT COLLATE \"C\"");
    let mut c = pg.conn()?;
    let mut tx = c.transaction()?;
    tx.execute("SELECT pg_advisory_xact_lock(7140513713)", &[])?;
    tx.batch_execute(&sql)?;
    tx.commit()?;
    Ok(())
}

fn db(app: &App, team: &str) -> Result<Db, ApiError> {
    db_with(app, team, "task-cache.sqlite", SCHEMA)
}

/// A session token: `qtc1_<hex of the JSON>_<hex of its MAC>` (no dots,
/// so it is never taken for a JWT).
pub const SESSION_PREFIX: &str = "qtc1_";
/// How long a session is good for: GitHub's longest job.
pub const SESSION_TTL_SECS: u64 = 6 * 3600;

/// What a session proves.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub team: String,
    pub repository: String,
    pub repo_key: String,
    /// The scope it writes, if it may write.
    pub write: Option<String>,
    /// The scopes it reads, in order.
    pub read: Vec<String>,
    /// The job's environment (`ns_of`).
    pub ns: String,
    /// Unix seconds.
    pub exp: u64,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn mac_key(app: &App) -> [u8; 32] {
    blake3::derive_key(
        "quod-server task-cache session v1",
        &app.settings.signing_key.to_bytes(),
    )
}

pub fn seal(key: &[u8; 32], s: &Session) -> String {
    let body = serde_json::to_vec(s).expect("a session serializes");
    let mac = blake3::keyed_hash(key, &body);
    format!("{SESSION_PREFIX}{}_{}", hex::encode(&body), mac.to_hex())
}

pub fn open(key: &[u8; 32], token: &str, now: u64) -> Result<Session, ApiError> {
    let bad = || ApiError::unauthorized("not a task-cache session of this server");
    let rest = token.strip_prefix(SESSION_PREFIX).ok_or_else(bad)?;
    let (body, mac) = rest.split_once('_').ok_or_else(bad)?;
    let body = hex::decode(body).map_err(|_| bad())?;
    let mac: [u8; 32] = hex::decode(mac)
        .ok()
        .and_then(|m| m.try_into().ok())
        .ok_or_else(bad)?;
    // blake3::Hash compares in constant time.
    if blake3::keyed_hash(key, &body) != blake3::Hash::from(mac) {
        return Err(bad());
    }
    let s: Session = serde_json::from_slice(&body).map_err(|_| bad())?;
    if s.exp <= now {
        return Err(ApiError::unauthorized(
            "the task-cache session expired; the next job gets a new one",
        ));
    }
    Ok(s)
}

/// The environment a job states, as one id. Unknown keys are kept (the
/// action decides what an environment is); values are capped.
pub fn ns_of(env: &std::collections::BTreeMap<String, String>) -> String {
    let mut h = blake3::Hasher::new();
    for (k, v) in env.iter().take(32) {
        h.update(crate::api::cap(k).as_bytes());
        h.update(b"\0");
        h.update(crate::api::cap(v).as_bytes());
        h.update(b"\0");
    }
    h.finalize().to_hex()[..32].to_string()
}

#[derive(Debug, Deserialize)]
pub struct SessionRequest {
    /// The job's environment: os, arch, node, image, ...
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct SessionResponse {
    pub token: String,
    pub expires_at: u64,
    pub read: Vec<String>,
    pub write: Option<String>,
}

/// `POST /v1/task-cache/session`: the job's OIDC token (or a static
/// runner token) for a session the task runner can hold all job long.
pub async fn session(
    State(app): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(req): Json<SessionRequest>,
) -> Result<Json<SessionResponse>, ApiError> {
    blocking(move || {
        // A delegated token states its own scope: it could claim `trusted`.
        if matches!(principal, Principal::Delegated { .. }) {
            return Err(ApiError::forbidden(
                "a delegated runner token cannot open a task-cache session; use the job's OIDC token",
            ));
        }
        let runner = App::runner_of(&principal, None)?;
        let s = Session {
            team: runner.team.clone(),
            repository: runner.repository.clone(),
            repo_key: repo_key(&runner)?,
            write: may_save(&runner).then(|| runner.write.clone()),
            read: read_scopes(&runner),
            ns: ns_of(&req.env),
            exp: now_secs() + SESSION_TTL_SECS,
        };
        tracing::info!(target: "quod", team = %s.team, repository = %s.repository, write = ?s.write, ns = %s.ns, "task-cache session");
        Ok(Json(SessionResponse {
            token: seal(&mac_key(&app), &s),
            expires_at: s.exp,
            read: s.read.clone(),
            write: s.write.clone(),
        }))
    })
    .await
}

fn session_of(app: &App, headers: &HeaderMap) -> Result<Session, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")))
        .ok_or_else(|| ApiError::unauthorized("no bearer token"))?;
    open(&mac_key(app), token.trim(), now_secs())
}

/// The tools whose artifacts are kept apart (one hash space each).
#[derive(Clone, Copy)]
enum Tool {
    Turbo,
    Nx,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Turbo => "turbo",
            Tool::Nx => "nx",
        }
    }
}

struct Found {
    scope: String,
    blob: Hash,
    size: u64,
    duration: Option<i64>,
    tag: Option<String>,
}

fn check_hash(h: &str) -> Result<(), ApiError> {
    if !crate::valid_id(h) {
        return Err(ApiError::bad_request("invalid artifact hash"));
    }
    Ok(())
}

/// The artifact a session reads for `hash`: from its first read scope
/// that has one.
fn find(app: &App, s: &Session, tool: Tool, hash: &str) -> Result<Option<Found>, ApiError> {
    let d = db(app, &s.team)?;
    let rows = d.query(
        "SELECT scope, blob, size, duration, tag FROM task_cache WHERE team = $1 AND repo_key = $2 AND ns = $3 AND tool = $4 AND hash = $5",
        &[
            t(s.team.as_str()),
            t(s.repo_key.as_str()),
            t(s.ns.as_str()),
            t(tool.name()),
            t(hash),
        ],
        &[Ty::T, Ty::T, Ty::I, Ty::I, Ty::T],
    )?;
    for scope in &s.read {
        if let Some(r) = rows.iter().find(|r| r[0].t().as_deref() == Some(scope)) {
            let blob: Hash = r[1]
                .t()
                .unwrap_or_default()
                .parse()
                .map_err(|_| ApiError::internal("a task-cache row with a bad blob hash"))?;
            return Ok(Some(Found {
                scope: scope.clone(),
                blob,
                size: r[2].i().unwrap_or(0) as u64,
                duration: r[3].i(),
                tag: r[4].t(),
            }));
        }
    }
    Ok(None)
}

async fn find_async(
    app: &AppState,
    headers: &HeaderMap,
    tool: Tool,
    hash: String,
) -> Result<(Session, Option<Found>), ApiError> {
    check_hash(&hash)?;
    let (app, headers) = (app.clone(), headers.clone());
    blocking(move || {
        let s = session_of(&app, &headers)?;
        let f = find(&app, &s, tool, &hash)?;
        Ok((s, f))
    })
    .await
}

fn found_headers(resp: &mut Response, f: &Found) {
    let h = resp.headers_mut();
    if let Some(d) = f.duration {
        h.insert("x-artifact-duration", HeaderValue::from(d));
    }
    if let Some(tag) = f.tag.as_deref().and_then(|t| HeaderValue::from_str(t).ok()) {
        h.insert("x-artifact-tag", tag);
    }
    if let Ok(v) = HeaderValue::from_str(&f.scope) {
        h.insert("x-quod-scope", v);
    }
}

async fn serve(app: &AppState, team: &str, f: &Found) -> Result<Response, ApiError> {
    let (body, len) = match app.store.cas(team)? {
        TeamBlobs::Local(cas) => {
            let file = tokio::fs::File::open(cas.blob_path(&f.blob))
                .await
                .map_err(|_| ApiError::internal("a task-cache artifact's blob is missing"))?;
            (
                Body::from_stream(tokio_util::io::ReaderStream::new(file)),
                Some(f.size),
            )
        }
        TeamBlobs::S3(blobs) => crate::blobs::s3_body(blobs, f.blob).await?,
    };
    let mut resp = (
        [("content-type", "application/octet-stream".to_string())],
        body,
    )
        .into_response();
    if let Some(len) = len {
        resp.headers_mut()
            .insert("content-length", HeaderValue::from(len));
    }
    found_headers(&mut resp, f);
    Ok(resp)
}

async fn get(app: AppState, headers: HeaderMap, tool: Tool, hash: String) -> Result<Response, ApiError> {
    let (s, f) = find_async(&app, &headers, tool, hash).await?;
    match f {
        Some(f) => serve(&app, &s.team, &f).await,
        None => Err(ApiError::not_found("no such artifact")),
    }
}

/// Stores the body as the artifact for `hash` in the session's write
/// scope. Ok(false): that scope already had one (it stays).
async fn put(
    app: AppState,
    headers: HeaderMap,
    tool: Tool,
    hash: String,
    body: Body,
) -> Result<bool, ApiError> {
    check_hash(&hash)?;
    let (a, h) = (app.clone(), headers.clone());
    let s = blocking(move || session_of(&a, &h)).await?;
    let Some(scope) = s.write.clone() else {
        return Err(ApiError::forbidden(
            "this job runs on input someone else controls; it reads the default branch's task cache but does not write it",
        ));
    };
    let exists = {
        let (a, s, hash, scope) = (app.clone(), s.clone(), hash.clone(), scope.clone());
        blocking(move || {
            let d = db(&a, &s.team)?;
            Ok(!d
                .query(
                    "SELECT size FROM task_cache WHERE team = $1 AND repo_key = $2 AND ns = $3 AND tool = $4 AND scope = $5 AND hash = $6",
                    &[t(s.team.as_str()), t(s.repo_key.as_str()), t(s.ns.as_str()), t(tool.name()), t(scope.as_str()), t(hash.as_str())],
                    &[Ty::I],
                )?
                .is_empty())
        })
        .await?
    };
    if exists {
        return Ok(false);
    }
    let dir = app.store.uploads(&s.team)?;
    crate::blobs::check_free_space(&dir)?;
    let tmp = tempfile::Builder::new()
        .prefix("task.put.")
        .tempfile_in(&dir)
        .map_err(ApiError::internal)?
        .into_temp_path();
    crate::blobs::append(&tmp, body, app.settings.max_blob).await?;
    let duration = headers
        .get("x-artifact-duration")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<i64>().ok());
    let tag = headers
        .get("x-artifact-tag")
        .and_then(|v| v.to_str().ok())
        .map(crate::api::cap);
    let path = tmp.to_path_buf();
    let inserted = blocking(move || {
        let size = std::fs::metadata(&path).map_err(ApiError::internal)?.len();
        let (blob, _) = app
            .store
            .cas(&s.team)?
            .put_blob_file(&path)
            .map_err(ApiError::internal)?;
        let d = db(&app, &s.team)?;
        let rows = d.query(
            "INSERT INTO task_cache (team, repo_key, ns, tool, scope, hash, blob, size, duration, tag, at, repository)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT DO NOTHING RETURNING size",
            &[
                t(s.team.as_str()), t(s.repo_key.as_str()), t(s.ns.as_str()), t(tool.name()),
                t(scope.as_str()), t(hash.as_str()), t(blob.to_string()), i(size as i64),
                V::I(duration), ot(tag), t(now_rfc3339()), t(s.repository.as_str()),
            ],
            &[Ty::I],
        )?;
        tracing::info!(target: "quod", team = %s.team, repository = %s.repository, tool = tool.name(), scope = %scope, hash = %hash, size, new = !rows.is_empty(), "task-cache artifact stored");
        Ok(!rows.is_empty())
    })
    .await?;
    drop(tmp);
    Ok(inserted)
}

// turbo: https://turborepo.com/docs/core-concepts/remote-caching#self-hosting

pub async fn turbo_status(State(app): State<AppState>, headers: HeaderMap) -> Result<Json<serde_json::Value>, ApiError> {
    let a = app.clone();
    blocking(move || session_of(&a, &headers)).await?;
    Ok(Json(serde_json::json!({"status": "enabled"})))
}

pub async fn turbo_get(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(hash): UrlPath<String>,
) -> Result<Response, ApiError> {
    get(app, headers, Tool::Turbo, hash).await
}

pub async fn turbo_head(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(hash): UrlPath<String>,
) -> Result<Response, ApiError> {
    let (_, f) = find_async(&app, &headers, Tool::Turbo, hash).await?;
    match f {
        Some(f) => {
            let mut r = StatusCode::OK.into_response();
            found_headers(&mut r, &f);
            Ok(r)
        }
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}

pub async fn turbo_put(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(hash): UrlPath<String>,
    body: Body,
) -> Result<Response, ApiError> {
    put(app, headers, Tool::Turbo, hash, body).await?;
    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({"urls": []}))).into_response())
}

/// `POST /v8/artifacts`: which of `hashes` the session reads.
pub async fn turbo_query(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let hashes: Vec<String> = req["hashes"]
        .as_array()
        .map(|a| a.iter().filter_map(|h| h.as_str().map(String::from)).take(1000).collect())
        .unwrap_or_default();
    blocking(move || {
        let s = session_of(&app, &headers)?;
        let mut out = serde_json::Map::new();
        for h in hashes {
            let v = if crate::valid_id(&h) {
                match find(&app, &s, Tool::Turbo, &h)? {
                    Some(f) => serde_json::json!({"size": f.size, "taskDurationMs": f.duration.unwrap_or(0), "tag": f.tag}),
                    None => serde_json::Value::Null,
                }
            } else {
                serde_json::Value::Null
            };
            out.insert(h, v);
        }
        Ok(Json(serde_json::Value::Object(out)))
    })
    .await
}

/// `POST /v8/artifacts/events`: turbo's hit and miss events; not kept.
pub async fn turbo_events(State(app): State<AppState>, headers: HeaderMap) -> Result<StatusCode, ApiError> {
    let a = app.clone();
    blocking(move || session_of(&a, &headers)).await?;
    Ok(StatusCode::OK)
}

// Nx: the self-hosted remote cache OpenAPI spec (NX_SELF_HOSTED_REMOTE_CACHE_SERVER).

pub async fn nx_get(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(hash): UrlPath<String>,
) -> Result<Response, ApiError> {
    get(app, headers, Tool::Nx, hash).await
}

pub async fn nx_put(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(hash): UrlPath<String>,
    body: Body,
) -> Result<Response, ApiError> {
    if put(app, headers, Tool::Nx, hash, body).await? {
        Ok(StatusCode::OK.into_response())
    } else {
        // The spec's answer to a second write of a hash.
        Ok(StatusCode::CONFLICT.into_response())
    }
}

pub fn router() -> axum::Router<AppState> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/v8/artifacts/status", get(turbo_status))
        .route("/v8/artifacts/events", post(turbo_events))
        .route("/v8/artifacts", post(turbo_query))
        .route(
            "/v8/artifacts/{hash}",
            get(turbo_get).head(turbo_head).put(turbo_put),
        )
        .route("/nx/v1/cache/{hash}", get(nx_get).put(nx_put))
        // Uploads stream to disk with the server's own cap (QUOD_MAX_BLOB_BYTES).
        .layer(axum::extract::DefaultBodyLimit::disable())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::TRUSTED;

    fn s() -> Session {
        Session {
            team: "acme".into(),
            repository: "acme/app".into(),
            repo_key: "github-7".into(),
            write: Some(TRUSTED.into()),
            read: vec![TRUSTED.into()],
            ns: "n".into(),
            exp: 1000,
        }
    }

    #[test]
    fn sessions_open_only_with_their_key_unchanged_and_in_time() {
        let k = [7u8; 32];
        let tok = seal(&k, &s());
        assert!(!tok.contains('.'));
        assert_eq!(open(&k, &tok, 999).unwrap(), s());
        assert!(open(&k, &tok, 1000).is_err(), "expired");
        assert!(open(&[8u8; 32], &tok, 1).is_err(), "another key");
        // A changed body (another team) with the old MAC.
        let mut other = s();
        other.team = "beta".into();
        let forged = seal(&[9u8; 32], &other);
        let (_, mac) = tok.rsplit_once('_').unwrap();
        let (body, _) = forged.rsplit_once('_').unwrap();
        assert!(open(&k, &format!("{body}_{mac}"), 1).is_err(), "forged");
        assert!(open(&k, "qtc1_zz_00", 1).is_err());
        assert!(open(&k, "eyJ.a.b", 1).is_err());
    }

    #[test]
    fn the_environment_is_part_of_the_key() {
        let mut a = std::collections::BTreeMap::new();
        a.insert("node".to_string(), "v22.1.0".to_string());
        let mut b = a.clone();
        b.insert("node".to_string(), "v20.1.0".to_string());
        assert_ne!(ns_of(&a), ns_of(&b));
        assert_eq!(ns_of(&a), ns_of(&a.clone()));
    }
}
