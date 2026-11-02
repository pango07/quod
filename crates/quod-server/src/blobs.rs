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

pub async fn missing(
    State(app): State<AppState>,
    headers: HeaderMap,
    axum::Json(req): axum::Json<BlobsRequest>,
) -> Result<axum::Json<BlobsResponse>, ApiError> {
    blocking(move || {
        let team = app.runner_team(&headers)?;
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
    let path = app.store.cas(&team)?.blob_path(&h);
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|_| ApiError::not_found("no such blob"))?;
    let len = file.metadata().await.map_err(ApiError::internal)?.len();
    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));
    Ok((
        [
            ("content-type", "application/octet-stream".to_string()),
            ("content-length", len.to_string()),
        ],
        body,
    )
        .into_response())
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
    let cas = app.store.cas(&team)?;
    if cas.has(&h) {
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

/// Streams `body` onto the end of `file`, failing past `max` total bytes.
async fn append(file: &Path, body: Body, max: u64) -> Result<u64, ApiError> {
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .await
        .map_err(ApiError::internal)?;
    let mut len = f.metadata().await.map_err(ApiError::internal)?.len();
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
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
    if app.store.cas(&team)?.has(&h) {
        return Ok(StatusCode::OK.into_response());
    }
    let dir = app.store.uploads(&team)?;
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
            .unwrap()
            .insert(k.clone())
            .then_some(Uploading(k))
    }
}

impl Drop for Uploading {
    fn drop(&mut self) {
        UPLOADING.lock().unwrap().remove(&self.0);
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
    if app.store.cas(&team)?.has(&h) {
        return Ok(StatusCode::OK.into_response());
    }
    let part = part_path(&app.store.uploads(&team)?, &h);
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
    if app.store.cas(&team)?.has(&h) {
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
