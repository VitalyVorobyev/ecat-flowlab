//! Streaming HTTP/TCP throughput and latency benchmark endpoints.
//!
//! These routes are mounted outside the HTTP compression layer so measured byte rates always
//! match actual payload bytes on the wire. The `/bytes/{n}` and `/upload` aliases match the
//! minimal HTTP/1.0 protocol used by `flowlab tcp-bench`.

use std::pin::Pin;
use std::sync::OnceLock;
use std::task::{Context, Poll};
use std::time::Instant;

use axum::body::Body;
use axum::extract::{Path as UrlPath, Query, Request};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use bytes::Bytes;
use http_body::{Body as HttpBody, Frame};
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use crate::{ApiError, AppState, blocking};

/// Largest payload `/api/bench/bytes/{n}` and `/bytes/{n}` will generate, bytes (1 GiB).
pub const MAX_BENCH_BYTES: u64 = 1 << 30;
/// Streaming chunk size for generated download payloads, bytes.
const CHUNK_BYTES: usize = 1 << 16;

static START: OnceLock<Instant> = OnceLock::new();

fn uptime_ns() -> u64 {
    let start = START.get_or_init(Instant::now);
    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Monotonic server timestamp returned by `GET /api/bench/ping`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PingResponse {
    /// Monotonic server uptime at response time, nanoseconds.
    pub server_time_ns: u64,
}

/// Result of a streaming upload benchmark (`POST /api/bench/upload`).
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct UploadBenchResult {
    /// Total payload bytes received.
    pub bytes: u64,
    /// Lower-case hex SHA-256 of the received bytes.
    pub sha256: String,
    /// Elapsed time on the server from the first body frame to EOF, seconds.
    pub server_seconds: f64,
    /// Effective receive rate on the server, bytes/s.
    pub bytes_per_s: f64,
}

#[derive(Deserialize)]
pub(crate) struct SeedQuery {
    #[serde(default = "default_seed")]
    seed: u64,
}

fn default_seed() -> u64 {
    1
}

struct PayloadBody {
    seed: u64,
    offset: u64,
    total: u64,
}

impl HttpBody for PayloadBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if self.offset >= self.total {
            return Poll::Ready(None);
        }
        let n = CHUNK_BYTES.min((self.total - self.offset) as usize);
        let mut buf = vec![0u8; n];
        flowlab_mailbox::payload_bytes(self.seed, self.offset, &mut buf);
        self.offset += n as u64;
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(buf)))))
    }

    fn is_end_stream(&self) -> bool {
        self.offset >= self.total
    }

    fn size_hint(&self) -> http_body::SizeHint {
        http_body::SizeHint::with_exact(self.total.saturating_sub(self.offset))
    }
}

async fn ping() -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(PingResponse { server_time_ns: uptime_ns() })).into_response()
}

async fn download(UrlPath(bytes): UrlPath<u64>, Query(q): Query<SeedQuery>) -> Result<Response, ApiError> {
    if bytes > MAX_BENCH_BYTES {
        return Err(ApiError::with_status(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("requested {bytes} B exceeds maximum {MAX_BENCH_BYTES} B"),
        ));
    }
    let seed = q.seed;
    let sha = blocking(move || Ok(flowlab_mailbox::payload_sha256(seed, bytes))).await?;
    let body = Body::new(PayloadBody { seed, offset: 0, total: bytes });
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.to_string())
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-sha256", sha)
        .body(body)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(resp)
}

async fn consume_upload(req: Request) -> Result<UploadBenchResult, ApiError> {
    let mut body = req.into_body();
    let mut hasher = Sha256::new();
    let mut bytes: u64 = 0;
    let mut started: Option<Instant> = None;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|e| ApiError::bad_request(format!("upload stream error: {e}")))?;
        if let Ok(data) = frame.into_data() {
            if started.is_none() {
                started = Some(Instant::now());
            }
            bytes = bytes.saturating_add(data.len() as u64);
            hasher.update(&data);
        }
    }
    let server_seconds = started.map_or(0.0, |t| t.elapsed().as_secs_f64());
    let bytes_per_s = if server_seconds > 0.0 { bytes as f64 / server_seconds } else { 0.0 };
    let sha256 = flowlab_core::hex(&hasher.finalize());
    Ok(UploadBenchResult { bytes, sha256, server_seconds, bytes_per_s })
}

async fn upload_json(req: Request) -> Result<Response, ApiError> {
    let result = consume_upload(req).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(result)).into_response())
}

async fn upload_text(req: Request) -> Result<Response, ApiError> {
    let r = consume_upload(req).await?;
    let body = format!("{} {} {:.6}\n", r.bytes, r.sha256, r.server_seconds);
    Ok(([(header::CONTENT_TYPE, "text/plain"), (header::CACHE_CONTROL, "no-store")], body).into_response())
}

/// Throughput and latency benchmark routes (uncompressed).
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/bench/ping", get(ping))
        .route("/api/bench/bytes/{bytes}", get(download))
        .route("/api/bench/upload", post(upload_json).put(upload_json))
        .route("/bytes/{bytes}", get(download))
        .route("/upload", put(upload_text).post(upload_text))
}
