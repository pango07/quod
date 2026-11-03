//! quod-server: one ledger and one result store for every runner of a
//! team, spoken to over quod.v1 (quod-wire). Trust transitions happen
//! here, inside the server's own transactions, with the same quod-trust
//! code a local ledger runs, so two runners can never both "win".
//!
//! Storage (`store`) is either SQLite plus a directory per team on one
//! persistent volume (self-hosted), or Postgres (`pg`) plus an
//! S3-compatible bucket (`s3`) for Quod Cloud, chosen by the environment
//! (`config::cloud_env`); `import` moves the first into the second. One
//! instance serves a data dir (`boot`: the data dir checks, the instance
//! lock, the signing key). docs/cloud-ledger.md has the design and the
//! Railway runbooks.

pub mod api;
pub mod auth;
pub mod blobs;
pub mod boot;
pub mod config;
pub mod dash;
pub mod import;
pub mod pg;
pub mod s3;
pub mod store;
pub mod testreplay;
pub mod warmup;

use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use quod_cas::Hash;
use quod_wire::{ErrorBody, PROTOCOL, PROTOCOL_HEADER};

use crate::auth::{Auth, Principal, Runner};
use crate::config::Settings;
use crate::store::Store;

/// Ids in paths (team, repo slug, step key, scope): `^[A-Za-z0-9._-]{1,120}$`,
/// never `.` or `..`.
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 120
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

pub struct App {
    pub settings: Settings,
    pub auth: Auth,
    pub store: Store,
}

pub type AppState = Arc<App>;

impl App {
    pub fn new(settings: Settings) -> App {
        App {
            auth: Auth::new(&settings.doc, settings.workos_api_key.clone()),
            store: match &settings.cloud {
                Some(c) => Store::cloud(&settings.data_dir, c.clone()),
                None => Store::new(&settings.data_dir),
            },
            settings,
        }
    }

    /// The runner a request comes from (blocking: OIDC may fetch the JWKS).
    pub fn runner(
        &self,
        headers: &HeaderMap,
        hint: Option<&quod_wire::ScopeHint>,
    ) -> Result<Runner, ApiError> {
        App::runner_of(&self.auth.authenticate(headers)?, hint)
    }

    /// The runner an authenticated principal is, for this request's scope
    /// hint.
    pub fn runner_of(
        p: &Principal,
        hint: Option<&quod_wire::ScopeHint>,
    ) -> Result<Runner, ApiError> {
        match p {
            Principal::Runner(r) => Ok((**r).clone()),
            Principal::Delegated { team } => Auth::delegated(team, hint),
            Principal::Viewer(_) => {
                Err(ApiError::forbidden("a viewer token cannot act as a runner"))
            }
        }
    }

    /// The team a runner request may touch blobs of.
    pub fn runner_team(&self, headers: &HeaderMap) -> Result<String, ApiError> {
        App::runner_team_of(&self.auth.authenticate(headers)?)
    }

    pub fn runner_team_of(p: &Principal) -> Result<String, ApiError> {
        match p {
            Principal::Runner(r) => Ok(r.team.clone()),
            Principal::Delegated { team } => Ok(team.clone()),
            Principal::Viewer(_) => {
                Err(ApiError::forbidden("a viewer token cannot act as a runner"))
            }
        }
    }
}

/// How long a JSON request may take, end to end.
pub const JSON_DEADLINE: std::time::Duration = std::time::Duration::from_secs(120);
/// How long a client may take to send a request's headers.
pub const HEADER_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The runner JSON endpoints authenticate from the headers before anything
/// reads the body: an unauthenticated caller is refused without the server
/// buffering (or even receiving) up to `QUOD_MAX_JSON_BYTES`. The principal
/// goes to the handler as an extension. The whole request then has
/// `JSON_DEADLINE` to finish.
async fn runner_auth(State(app): State<AppState>, mut req: Request, next: Next) -> Response {
    let headers = req.headers().clone();
    let a = app.clone();
    match blocking(move || a.auth.authenticate(&headers)).await {
        Ok(p) => {
            req.extensions_mut().insert(p);
        }
        Err(e) => return e.into_response(),
    }
    match tokio::time::timeout(JSON_DEADLINE, next.run(req)).await {
        Ok(r) => r,
        Err(_) => ApiError::new(
            StatusCode::REQUEST_TIMEOUT,
            "timeout",
            "the request took too long",
        )
        .into_response(),
    }
}

/// An error answer: a status and quod.v1's error body.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub body: ErrorBody,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &str, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            body: ErrorBody {
                error: code.into(),
                message: message.into(),
                missing: vec![],
            },
        }
    }
    pub fn unauthorized(m: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", m)
    }
    pub fn forbidden(m: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::FORBIDDEN, "forbidden", m)
    }
    pub fn not_found(m: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, "not_found", m)
    }
    pub fn bad_request(m: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, "bad_request", m)
    }
    pub fn conflict(m: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::CONFLICT, "conflict", m)
    }
    pub fn not_implemented(m: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::NOT_IMPLEMENTED, "not_implemented", m)
    }
    pub fn missing_blobs(missing: Vec<Hash>) -> ApiError {
        let mut e = ApiError::new(
            StatusCode::CONFLICT,
            "missing_blobs",
            format!(
                "{} blob(s) of the result are not on the server; upload them first",
                missing.len()
            ),
        );
        e.body.missing = missing;
        e
    }
    pub fn internal(e: impl std::fmt::Display) -> ApiError {
        tracing::error!(target: "quod", error = %e, "internal error");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error",
        )
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> ApiError {
        ApiError::internal(format!("{e:#}"))
    }
}

impl From<quod_trust::TrustError> for ApiError {
    fn from(e: quod_trust::TrustError) -> ApiError {
        match e {
            quod_trust::TrustError::BadPolicy(m) => ApiError::bad_request(m),
            e => ApiError::internal(e),
        }
    }
}

impl From<quod_ledger::LedgerError> for ApiError {
    fn from(e: quod_ledger::LedgerError) -> ApiError {
        ApiError::internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, axum::Json(self.body)).into_response()
    }
}

/// Runs blocking work (SQLite, the local CAS, a JWKS fetch) off the
/// async workers.
pub async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(ApiError::internal)?
}

/// Every `/v1` request names the protocol; every answer does too.
async fn protocol(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    let named = req.headers().get(PROTOCOL_HEADER).map(|v| v.as_bytes());
    // A browser's EventSource cannot set headers; the dashboard's server
    // side re-serves the stream, but a missing header on the stream itself
    // is not refused.
    let exempt = path == "/v1/info" || path == "/v1/keys" || path.ends_with("/events");
    if path.starts_with("/v1/")
        && named != Some(PROTOCOL.as_bytes())
        && !(exempt && named.is_none())
    {
        return ApiError::new(
            StatusCode::BAD_REQUEST,
            "protocol",
            format!("this server speaks {PROTOCOL}; send `{PROTOCOL_HEADER}: {PROTOCOL}`"),
        )
        .into_response();
    }
    let mut resp = next.run(req).await;
    resp.headers_mut()
        .insert(PROTOCOL_HEADER, HeaderValue::from_static(PROTOCOL));
    resp
}

/// Liveness and the metadata a client or an operator checks: the protocol,
/// the build, and the public key receipts are signed with (never the
/// private key).
async fn healthz(State(app): State<AppState>) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "protocol": PROTOCOL,
        "version": env!("CARGO_PKG_VERSION"),
        "keys": [quod_wire::PublicKey::of(&app.settings.signing_key.verifying_key())],
    }))
}

async fn info(State(app): State<AppState>) -> axum::Json<quod_wire::Info> {
    axum::Json(quod_wire::Info {
        protocol: PROTOCOL.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        signing_key: hex::encode(app.settings.signing_key.verifying_key().to_bytes()),
    })
}

/// The public keys clients pin (`QUOD_SERVER_KEY`).
async fn keys(State(app): State<AppState>) -> axum::Json<quod_wire::Keys> {
    axum::Json(quod_wire::Keys {
        protocol: PROTOCOL.into(),
        keys: vec![quod_wire::PublicKey::of(
            &app.settings.signing_key.verifying_key(),
        )],
    })
}

pub fn router(app: AppState) -> Router {
    let max_json = app.settings.max_json;
    let runner = Router::new()
        .route("/v1/claim", post(api::claim))
        .route("/v1/commit", post(api::commit))
        .route("/v1/divergence", post(api::divergence))
        .route("/v1/incomplete", post(api::incomplete))
        .route("/v1/runs", post(api::runs))
        .route("/v1/step-keys", post(api::step_keys))
        .route("/v1/blobs/missing", post(blobs::missing))
        .route("/v1/blobs/pack", post(testreplay::pack))
        .route("/v1/test-ledgers/lookup", post(testreplay::lookup))
        .route("/v1/test-ledgers", post(testreplay::save))
        .route("/v1/test-runs", post(testreplay::report))
        .layer(DefaultBodyLimit::max(max_json))
        .layer(middleware::from_fn_with_state(app.clone(), runner_auth));
    let blob = Router::new()
        .route(
            "/v1/blobs/{hash}",
            get(blobs::get_blob)
                .head(blobs::head_blob)
                .put(blobs::put_blob)
                .patch(blobs::patch_blob),
        )
        // Streamed to disk with the server's own cap (QUOD_MAX_BLOB_BYTES).
        .layer(DefaultBodyLimit::disable());
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/info", get(info))
        .route("/v1/keys", get(keys))
        .merge(runner)
        .merge(blob)
        .merge(dash::router())
        .layer(middleware::from_fn(protocol))
        .with_state(app)
}

/// Accepts connections until `shutdown` resolves. Each connection is
/// served by hyper with a header-read timeout (axum::serve sets no timer,
/// so a client that never finishes its headers would hold a connection
/// forever). On shutdown it stops accepting and returns at once: open
/// connections, event streams included, are dropped with the runtime
/// (`main` gives in-flight blocking work a few seconds), so a redeploy's
/// new instance is not kept waiting on the lock by a held SSE stream.
async fn accept_loop(
    listener: tokio::net::TcpListener,
    app: AppState,
    shutdown: impl std::future::Future<Output = ()>,
) {
    use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
    use hyper_util::server::conn::auto::Builder;
    use hyper_util::service::TowerToHyperService;

    let svc = router(app);
    tokio::pin!(shutdown);
    loop {
        let stream = tokio::select! {
            _ = &mut shutdown => return,
            acc = listener.accept() => match acc {
                Ok((s, _)) => s,
                Err(e) => {
                    tracing::warn!(target: "quod", error = %e, "accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
            },
        };
        let svc = TowerToHyperService::new(svc.clone());
        tokio::spawn(async move {
            let mut b = Builder::new(TokioExecutor::new());
            b.http1()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_READ_TIMEOUT);
            let _ = b
                .serve_connection_with_upgrades(TokioIo::new(stream), svc)
                .await;
        });
    }
}

/// Removes upload staging files older than a day under every team, at
/// start and then hourly (an abandoned resumable upload would otherwise
/// keep its bytes on the volume forever).
fn spawn_upload_sweeper(app: &AppState) {
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            let a = app.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let n = blobs::sweep_uploads(&a.settings.data_dir, blobs::STALE_UPLOAD);
                if n > 0 {
                    tracing::info!(target: "quod", removed = n, "stale uploads removed");
                }
            })
            .await;
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
        }
    });
}

/// Serves until ctrl-c or SIGTERM. The caller holds the instance lock
/// (`boot::Boot`) for as long as this runs.
pub async fn serve(settings: Settings) -> anyhow::Result<()> {
    let addr = format!("{}:{}", settings.bind, settings.port);
    let app = Arc::new(App::new(settings));
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(target: "quod", %addr, "quod-server listening");
    spawn_upload_sweeper(&app);
    accept_loop(listener, app, async {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM handler");
            tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
        }
        #[cfg(not(unix))]
        let _ = ctrl_c.await;
    })
    .await;
    tracing::info!(target: "quod", "quod-server stopping");
    Ok(())
}

/// A server on an ephemeral local port, for tests: its base URL and the
/// task serving it.
pub async fn spawn_local(
    settings: Settings,
) -> anyhow::Result<(String, tokio::task::JoinHandle<()>)> {
    std::fs::create_dir_all(&settings.data_dir)?;
    let app = Arc::new(App::new(settings));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}", listener.local_addr()?);
    let h = tokio::spawn(accept_loop(listener, app, std::future::pending()));
    Ok((url, h))
}
