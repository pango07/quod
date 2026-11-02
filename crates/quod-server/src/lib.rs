//! quod-server: one ledger and one result store for every runner of a
//! team, spoken to over quod.v1 (quod-wire). Trust transitions happen
//! here, inside the server's own transactions, with the same quod-trust
//! code a local ledger runs, so two runners can never both "win".
//!
//! Storage in this build is SQLite plus a directory per team (`store`) on
//! one persistent volume, served by one instance (`boot`: the data dir
//! checks, the instance lock, the signing key). docs/cloud-ledger.md has
//! the design, the go-live on Railway, and the Postgres and object-store
//! backends that come next.

pub mod api;
pub mod auth;
pub mod blobs;
pub mod boot;
pub mod config;
pub mod dash;
pub mod store;

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
            store: Store::new(&settings.data_dir),
            settings,
        }
    }

    /// The runner a request comes from (blocking: OIDC may fetch the JWKS).
    pub fn runner(
        &self,
        headers: &HeaderMap,
        hint: Option<&quod_wire::ScopeHint>,
    ) -> Result<Runner, ApiError> {
        match self.auth.authenticate(headers)? {
            Principal::Runner(r) => Ok(*r),
            Principal::Delegated { team } => Auth::delegated(&team, hint),
            Principal::Viewer(_) => {
                Err(ApiError::forbidden("a viewer token cannot act as a runner"))
            }
        }
    }

    /// The team a runner request may touch blobs of.
    pub fn runner_team(&self, headers: &HeaderMap) -> Result<String, ApiError> {
        match self.auth.authenticate(headers)? {
            Principal::Runner(r) => Ok(r.team),
            Principal::Delegated { team } => Ok(team),
            Principal::Viewer(_) => {
                Err(ApiError::forbidden("a viewer token cannot act as a runner"))
            }
        }
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
        .layer(DefaultBodyLimit::max(max_json));
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

/// Serves until ctrl-c or SIGTERM. The caller holds the instance lock
/// (`boot::Boot`) for as long as this runs.
pub async fn serve(settings: Settings) -> anyhow::Result<()> {
    let addr = format!("{}:{}", settings.bind, settings.port);
    let app = Arc::new(App::new(settings));
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(target: "quod", %addr, "quod-server listening");
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async {
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
        .await?;
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
    let h = tokio::spawn(async move {
        let _ = axum::serve(listener, router(app)).await;
    });
    Ok((url, h))
}
