//! Seqr mailbox library: router construction and HTTP handlers.
//!
//! Split out from `main.rs` so the full router can be exercised by integration tests
//! with real signatures, not just the storage layer in isolation.

pub mod config;
pub mod store;

use std::io::Write;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};

use seqr_protocol::mailbox::{
    AckRequest, LogRequest, PullRequest, PullResponse, PushRequest, PushResponse,
};

use config::Config;
use store::{is_hex, Store};

pub struct AppState {
    pub store: Store,
    pub config: Config,
}

pub type Shared = Arc<AppState>;

/// Assemble the router over a shared application state.
pub fn build_router(state: Shared) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/push", post(push))
        .route("/v1/pull", post(pull))
        .route("/v1/ack", post(ack))
        .route("/v1/log", post(client_log).get(read_log))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

/// First 8 chars of a hex id, for compact logging.
fn short(hex: &str) -> &str {
    &hex[..hex.len().min(8)]
}

/// Append a line to the debug log (`<data_dir>/debug.log`). Best-effort.
fn dlog(st: &Shared, line: &str) {
    let path = st.config.data_dir.join("debug.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{}] {}", now_secs(), line);
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Reject requests whose timestamp drifts too far from the server clock (replay guard).
pub fn fresh(ts_millis: u64, skew_secs: u64) -> bool {
    let ts_secs = ts_millis / 1000;
    let now = now_secs();
    let lo = now.saturating_sub(skew_secs);
    let hi = now + skew_secs;
    ts_secs >= lo && ts_secs <= hi
}

/// Verify an Ed25519 signature over `msg` by the key `identity_hex`.
pub fn verify_identity(identity_hex: &str, msg: &[u8], signature_hex: &str) -> bool {
    if !is_hex(identity_hex, 64) || !is_hex(signature_hex, 128) {
        return false;
    }
    let (Ok(id), Ok(sig)) = (hex::decode(identity_hex), hex::decode(signature_hex)) else {
        return false;
    };
    let Ok(id): Result<[u8; 32], _> = id.try_into() else { return false };
    let Ok(sig): Result<[u8; 64], _> = sig.try_into() else { return false };
    seqr_crypto::sign::verify_raw(&id, msg, &sig).is_ok()
}

async fn push(
    State(st): State<Shared>,
    Json(req): Json<PushRequest>,
) -> Result<Json<PushResponse>, StatusCode> {
    if !is_hex(&req.to, 64) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if req.payload.is_empty() || req.payload.len() > st.config.max_payload {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let id = st.store.push(&req.to, &req.payload).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    dlog(&st, &format!("PUSH   to={} bytes={} id={}", short(&req.to), req.payload.len(), id));
    Ok(Json(PushResponse { id }))
}

async fn pull(
    State(st): State<Shared>,
    Json(req): Json<PullRequest>,
) -> Result<Json<PullResponse>, StatusCode> {
    if !fresh(req.ts, st.config.clock_skew_secs) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let msg = PullRequest::signing_bytes(&req.identity, req.ts);
    if !verify_identity(&req.identity, &msg, &req.signature) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let messages = st
        .store
        .pull(&req.identity, st.config.pull_limit)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    // Log every poll (returned count) so we can see which identities are actively
    // polling — invaluable for diagnosing delivery. (Truncate the log when done.)
    dlog(&st, &format!("PULL   identity={} returned={}", short(&req.identity), messages.len()));
    Ok(Json(PullResponse { messages }))
}

async fn ack(
    State(st): State<Shared>,
    Json(req): Json<AckRequest>,
) -> Result<StatusCode, StatusCode> {
    if !fresh(req.ts, st.config.clock_skew_secs) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let msg = AckRequest::signing_bytes(&req.identity, req.ts, &req.ids);
    if !verify_identity(&req.identity, &msg, &req.signature) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    st.store.ack(&req.identity, &req.ids).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    dlog(&st, &format!("ACK    identity={} ids={}", short(&req.identity), req.ids.len()));
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/log` — append a client diagnostic line.
async fn client_log(State(st): State<Shared>, Json(req): Json<LogRequest>) -> StatusCode {
    // Truncate to keep lines bounded.
    let msg: String = req.msg.chars().take(500).collect();
    let tag: String = req.tag.chars().take(60).collect();
    dlog(&st, &format!("CLIENT {tag} | {msg}"));
    StatusCode::NO_CONTENT
}

/// `GET /v1/log` — return the debug log as plain text (debugging only).
async fn read_log(State(st): State<Shared>) -> String {
    std::fs::read_to_string(st.config.data_dir.join("debug.log")).unwrap_or_default()
}
