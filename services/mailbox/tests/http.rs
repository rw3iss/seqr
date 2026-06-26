//! End-to-end HTTP tests driving the real router with genuine Ed25519 signatures.
//! Exercises the full push -> pull -> ack lifecycle and the auth rejections.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use seqr_crypto::keys::Identity;
use seqr_crypto::sign::sign;
use seqr_mailbox::config::Config;
use seqr_mailbox::store::Store;
use seqr_mailbox::{build_router, AppState};
use seqr_protocol::mailbox::{
    AckRequest, PullRequest, PullResponse, PushRequest, PushResponse,
};

fn now_millis() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

fn test_state() -> Arc<AppState> {
    let mut dir = std::env::temp_dir();
    dir.push(format!("seqr-http-test-{}", hex::encode(seqr_crypto::group::generate_group_key())));
    let config = Config {
        bind: "0.0.0.0:0".into(),
        data_dir: dir,
        max_payload: 131072,
        clock_skew_secs: 120,
        pull_limit: 256,
    };
    let store = Store::new(&config.data_dir).unwrap();
    Arc::new(AppState { store, config, seen: std::sync::Mutex::new(std::collections::HashMap::new()) })
}

async fn post_json(state: Arc<AppState>, path: &str, body: &impl serde::Serialize) -> (StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap();
    let resp = build_router(state).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, bytes)
}

#[tokio::test]
async fn full_lifecycle() {
    let state = test_state();
    let recipient = Identity::generate();
    let rid = hex::encode(recipient.public().signing_public);

    // Push two ciphertext frames (open endpoint).
    let (s1, b1) = post_json(
        state.clone(),
        "/v1/push",
        &PushRequest { to: rid.clone(), payload: "ciphertext-A".into() },
    )
    .await;
    assert_eq!(s1, StatusCode::OK);
    let _: PushResponse = serde_json::from_slice(&b1).unwrap();

    let (s2, _) = post_json(
        state.clone(),
        "/v1/push",
        &PushRequest { to: rid.clone(), payload: "ciphertext-B".into() },
    )
    .await;
    assert_eq!(s2, StatusCode::OK);

    // Pull with a valid signature.
    let ts = now_millis();
    let sig = hex::encode(sign(&recipient.signing_key, &PullRequest::signing_bytes(&rid, ts)));
    let (s3, b3) = post_json(
        state.clone(),
        "/v1/pull",
        &PullRequest { identity: rid.clone(), ts, signature: sig },
    )
    .await;
    assert_eq!(s3, StatusCode::OK);
    let pulled: PullResponse = serde_json::from_slice(&b3).unwrap();
    assert_eq!(pulled.messages.len(), 2);
    assert_eq!(pulled.messages[0].payload, "ciphertext-A"); // oldest first

    // Ack the first; it should disappear.
    let ts2 = now_millis();
    let ids = vec![pulled.messages[0].id.clone()];
    let asig = hex::encode(sign(&recipient.signing_key, &AckRequest::signing_bytes(&rid, ts2, &ids)));
    let (s4, _) = post_json(
        state.clone(),
        "/v1/ack",
        &AckRequest { identity: rid.clone(), ts: ts2, signature: asig, ids },
    )
    .await;
    assert_eq!(s4, StatusCode::NO_CONTENT);

    let ts3 = now_millis();
    let sig3 = hex::encode(sign(&recipient.signing_key, &PullRequest::signing_bytes(&rid, ts3)));
    let (_, b5) = post_json(
        state.clone(),
        "/v1/pull",
        &PullRequest { identity: rid.clone(), ts: ts3, signature: sig3 },
    )
    .await;
    let after: PullResponse = serde_json::from_slice(&b5).unwrap();
    assert_eq!(after.messages.len(), 1);
    assert_eq!(after.messages[0].payload, "ciphertext-B");

    let _ = std::fs::remove_dir_all(&state.config.data_dir);
}

#[tokio::test]
async fn pull_rejects_forged_signature() {
    let state = test_state();
    let victim = Identity::generate();
    let attacker = Identity::generate();
    let vid = hex::encode(victim.public().signing_public);

    // Attacker signs with their own key but claims the victim's identity.
    let ts = now_millis();
    let sig = hex::encode(sign(&attacker.signing_key, &PullRequest::signing_bytes(&vid, ts)));
    let (status, _) = post_json(
        state.clone(),
        "/v1/pull",
        &PullRequest { identity: vid, ts, signature: sig },
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let _ = std::fs::remove_dir_all(&state.config.data_dir);
}

#[tokio::test]
async fn pull_rejects_stale_timestamp() {
    let state = test_state();
    let id = Identity::generate();
    let hid = hex::encode(id.public().signing_public);
    let ts = now_millis() - 10_000_000; // far in the past
    let sig = hex::encode(sign(&id.signing_key, &PullRequest::signing_bytes(&hid, ts)));
    let (status, _) = post_json(
        state.clone(),
        "/v1/pull",
        &PullRequest { identity: hid, ts, signature: sig },
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let _ = std::fs::remove_dir_all(&state.config.data_dir);
}
