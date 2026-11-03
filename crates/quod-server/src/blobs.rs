//! Blobs over quod.v1: content-addressed, per team.
//!
//! `PUT /v1/blobs/{hash}` takes a whole blob; `PATCH` appends a chunk at
//! `upload-offset` and, with `upload-complete: true`, finishes it; `HEAD`
//! says whether the blob is there and, if an upload is in progress, how
//! much of it (`upload-offset`), so an interrupted upload resumes. Bodies
//! stream to disk and are hashed as they arrive; nothing is stored under
//! a hash it does not have, and a blob over `QUOD_MAX_BLOB_BYTES` is
//! refused. `GET` streams it back.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use quod_cas::{BlobStore, Hash};
use quod_wire::{BlobsRequest, BlobsResponse};

use crate::store::TeamBlobs;
use crate::{ApiError, AppState, blocking};

/// A hash in a path: 64 hex digits, with or without `b3:`.
pub fn parse(hash: &str) -> Result<Hash, ApiError> {
    format!("b3:{}", hash.strip_prefix("b3:").unwrap_or(hash))
        .parse()
        .map_err(|_| ApiError::bad_request("not a BLAKE3 hash"))
}

async fn team(app: &AppState, headers: &HeaderMap) -> Result<String, ApiError> {
    let (app, headers) = (app.clone(), headers.clone());
    blocking(move || app.runner_team(&headers)).await
}

/// Whether the team's store holds `h` (a HEAD on the bucket with Postgres
/// and S3, so off the async workers).
async fn has(app: &AppState, team: &str, h: &Hash) -> Result<bool, ApiError> {
    let (app, team, h) = (app.clone(), team.to_string(), *h);
    blocking(move || {
        app.store
            .cas(&team)?
            .has_blob(&h)
            .map_err(ApiError::internal)
    })
    .await
}

/// Streams an object-store blob: a blocking reader on its own thread,
/// chunks through a channel. The reader checks the bytes against the hash
/// and fails at the end if they differ, which cuts the response short (a
/// runner verifies every blob it fetches anyway).
pub(crate) async fn s3_body(
    blobs: crate::s3::S3Blobs,
    h: Hash,
) -> Result<(Body, Option<u64>), ApiError> {
    let opened = blocking(move || match blobs.open(&h) {
        Ok(r) => Ok(Some(r)),
        Err(quod_cas::CasError::Missing(_)) => Ok(None),
        Err(e) => Err(ApiError::internal(e)),
    })
    .await?;
    let Some((mut reader, len)) = opened else {
        return Err(ApiError::not_found("no such blob"));
    };
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Vec<u8>, std::io::Error>>(4);
    tokio::task::spawn_blocking(move || {
        loop {
            let mut buf = vec![0u8; 256 << 10];
            match reader.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => {
                    buf.truncate(n);
                    if tx.blocking_send(Ok(buf)).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let _ = tx.blocking_send(Err(e));
                    return;
                }
            }
        }
    });
    let stream =
        futures_util::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|c| (c, rx)) });
    Ok((Body::from_stream(stream), len))
}

pub async fn missing(
    State(app): State<AppState>,
    axum::Extension(principal): axum::Extension<crate::auth::Principal>,
    axum::Json(req): axum::Json<BlobsRequest>,
) -> Result<axum::Json<BlobsResponse>, ApiError> {
    blocking(move || {
        let team = crate::App::runner_team_of(&principal)?;
        let cas = app.store.cas(&team)?;
        let mut missing = vec![];
        for h in req.hashes {
            if !cas.has_blob(&h).map_err(ApiError::internal)? {
                missing.push(h);
            }
        }
        missing.sort();
        missing.dedup();
        Ok(axum::Json(BlobsResponse { missing }))
    })
    .await
}

pub async fn get_blob(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(hash): UrlPath<String>,
) -> Result<Response, ApiError> {
    let h = parse(&hash)?;
    let team = team(&app, &headers).await?;
    let (body, len) = match app.store.cas(&team)? {
        TeamBlobs::Local(cas) => {
            let file = tokio::fs::File::open(cas.blob_path(&h))
                .await
                .map_err(|_| ApiError::not_found("no such blob"))?;
            let len = file.metadata().await.map_err(ApiError::internal)?.len();
            (
                Body::from_stream(tokio_util::io::ReaderStream::new(file)),
                Some(len),
            )
        }
        TeamBlobs::S3(blobs) => s3_body(blobs, h).await?,
    };
    let mut resp = (
        [("content-type", "application/octet-stream".to_string())],
        body,
    )
        .into_response();
    if let Some(len) = len {
        resp.headers_mut()
            .insert("content-length", axum::http::HeaderValue::from(len));
    }
    Ok(resp)
}

fn part_path(dir: &Path, h: &Hash) -> PathBuf {
    dir.join(format!("{}.part", h.hex()))
}

pub async fn head_blob(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(hash): UrlPath<String>,
) -> Result<Response, ApiError> {
    let h = parse(&hash)?;
    let team = team(&app, &headers).await?;
    if has(&app, &team, &h).await? {
        return Ok(StatusCode::OK.into_response());
    }
    let part = part_path(&app.store.uploads(&team)?, &h);
    let offset = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    Ok((
        StatusCode::NOT_FOUND,
        [("upload-offset", offset.to_string())],
    )
        .into_response())
}

/// Staging files older than this are swept (`sweep_uploads`).
pub const STALE_UPLOAD: std::time::Duration = std::time::Duration::from_secs(24 * 3600);
/// How long an upload may go without sending a byte.
pub const CHUNK_READ_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
/// Uploads are refused while the volume has less free space than this...
pub const FREE_FLOOR_BYTES: u64 = 2 << 30;
/// ...or less than this share of its size, whichever is larger.
pub const FREE_FLOOR_SHARE: f64 = 0.10;

/// Removes `*.part` and `*.put.*` staging files older than `age` from
/// every team's uploads directory; returns how many.
pub fn sweep_uploads(data_dir: &Path, age: std::time::Duration) -> usize {
    let mut n = 0;
    let Ok(teams) = std::fs::read_dir(data_dir.join("teams")) else {
        return 0;
    };
    for t in teams.flatten() {
        let Ok(files) = std::fs::read_dir(t.path().join("uploads")) else {
            continue;
        };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            if !(name.ends_with(".part") || name.contains(".put.")) {
                continue;
            }
            let old = f
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|e| e >= age);
            if old && std::fs::remove_file(f.path()).is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// Refuses an upload when the data dir's filesystem is short of space:
/// below `FREE_FLOOR_BYTES` or `FREE_FLOOR_SHARE` of its size, whichever
/// is larger. The ledgers need room to commit what is already uploaded.
pub(crate) fn check_free_space(dir: &Path) -> Result<(), ApiError> {
    let Ok(s) = rustix::fs::statvfs(dir) else {
        return Ok(());
    };
    let free = s.f_bavail.saturating_mul(s.f_frsize);
    let total = s.f_blocks.saturating_mul(s.f_frsize);
    let floor = FREE_FLOOR_BYTES.max((total as f64 * FREE_FLOOR_SHARE) as u64);
    if free < floor {
        tracing::warn!(target: "quod", free, floor, "uploads refused: the volume is nearly full");
        return Err(ApiError::new(
            StatusCode::INSUFFICIENT_STORAGE,
            "insufficient_storage",
            "the server's volume is nearly full; uploads are refused until space is freed",
        ));
    }
    Ok(())
}

/// Streams `body` onto the end of `file`, failing past `max` total bytes
/// or when no data comes for `CHUNK_READ_DEADLINE`.
pub(crate) async fn append(file: &Path, body: Body, max: u64) -> Result<u64, ApiError> {
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .await
        .map_err(ApiError::internal)?;
    let mut len = f.metadata().await.map_err(ApiError::internal)?.len();
    let mut stream = body.into_data_stream();
    loop {
        let next = tokio::time::timeout(CHUNK_READ_DEADLINE, stream.next())
            .await
            .map_err(|_| {
                ApiError::new(StatusCode::REQUEST_TIMEOUT, "timeout", "the upload stalled")
            })?;
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|e| ApiError::bad_request(format!("body: {e}")))?;
        len += chunk.len() as u64;
        if len > max {
            drop(f);
            let _ = tokio::fs::remove_file(file).await;
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                format!("blobs are limited to {max} bytes"),
            ));
        }
        tokio::io::AsyncWriteExt::write_all(&mut f, &chunk)
            .await
            .map_err(ApiError::internal)?;
    }
    tokio::io::AsyncWriteExt::flush(&mut f)
        .await
        .map_err(ApiError::internal)?;
    Ok(len)
}

/// Hashes a finished upload and stores it if it is the blob it claims to
/// be; the staging file is removed either way.
fn promote(app: &AppState, team: &str, staged: &Path, want: &Hash) -> Result<(), ApiError> {
    let got = {
        let mut f = std::fs::File::open(staged).map_err(ApiError::internal)?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf).map_err(ApiError::internal)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Hash::from_blake3(hasher.finalize())
    };
    let out = if got != *want {
        Err(ApiError::bad_request(format!(
            "the bytes hash to {got}, not {want}"
        )))
    } else {
        app.store
            .cas(team)?
            .put_blob_file(staged)
            .map(|_| ())
            .map_err(ApiError::internal)
    };
    let _ = std::fs::remove_file(staged);
    out
}

pub async fn put_blob(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(hash): UrlPath<String>,
    body: Body,
) -> Result<Response, ApiError> {
    let h = parse(&hash)?;
    let team = team(&app, &headers).await?;
    if has(&app, &team, &h).await? {
        return Ok(StatusCode::OK.into_response());
    }
    let dir = app.store.uploads(&team)?;
    check_free_space(&dir)?;
    let tmp = tempfile::Builder::new()
        .prefix(&format!("{}.put.", h.hex()))
        .tempfile_in(&dir)
        .map_err(ApiError::internal)?
        .into_temp_path();
    append(&tmp, body, app.settings.max_blob).await?;
    let path = tmp.to_path_buf();
    let app2 = app.clone();
    blocking(move || promote(&app2, &team, &path, &h)).await?;
    drop(tmp);
    Ok(StatusCode::CREATED.into_response())
}

/// Staging files being appended to, by (team, hash): appends to one must
/// not interleave; uploads of different blobs never wait on each other.
static UPLOADING: std::sync::Mutex<BTreeSet<(String, Hash)>> =
    std::sync::Mutex::new(BTreeSet::new());

/// Holds a staging file for one request; released on drop, however the
/// request ends.
struct Uploading((String, Hash));

impl Uploading {
    fn take(team: &str, h: &Hash) -> Option<Uploading> {
        let k = (team.to_string(), *h);
        UPLOADING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(k.clone())
            .then_some(Uploading(k))
    }
}

impl Drop for Uploading {
    fn drop(&mut self) {
        UPLOADING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

pub async fn patch_blob(
    State(app): State<AppState>,
    headers: HeaderMap,
    UrlPath(hash): UrlPath<String>,
    body: Body,
) -> Result<Response, ApiError> {
    let h = parse(&hash)?;
    let offset: u64 = headers
        .get("upload-offset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| ApiError::bad_request("PATCH needs upload-offset"))?;
    let complete = headers
        .get("upload-complete")
        .is_some_and(|v| v.as_bytes() == b"true");
    let team = team(&app, &headers).await?;
    if has(&app, &team, &h).await? {
        return Ok(StatusCode::OK.into_response());
    }
    let uploads = app.store.uploads(&team)?;
    check_free_space(&uploads)?;
    let part = part_path(&uploads, &h);
    let have = || std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    // One chunk at a time per staging file; another request appending to
    // it now is a 409 with the offset so far, and the client retries.
    let Some(_guard) = Uploading::take(&team, &h) else {
        return Ok((
            StatusCode::CONFLICT,
            [("upload-offset", have().to_string())],
        )
            .into_response());
    };
    if has(&app, &team, &h).await? {
        return Ok(StatusCode::OK.into_response());
    }
    let have = have();
    if have != offset {
        return Ok((StatusCode::CONFLICT, [("upload-offset", have.to_string())]).into_response());
    }
    let len = append(&part, body, app.settings.max_blob).await?;
    if complete {
        let app2 = app.clone();
        let p = part.clone();
        blocking(move || promote(&app2, &team, &p, &h)).await?;
        return Ok(StatusCode::CREATED.into_response());
    }
    Ok((StatusCode::ACCEPTED, [("upload-offset", len.to_string())]).into_response())
}
