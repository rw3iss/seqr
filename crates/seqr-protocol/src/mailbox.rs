//! Mailbox protocol — the HTTP contract between a client and the VPS helper.
//!
//! The mailbox parks **ciphertext** for offline recipients. It cannot read anything
//! (the payloads are already E2E-sealed) and holds no accounts. Two authenticated
//! facts matter:
//! - **Pull** is authenticated: a recipient proves ownership of its identity by
//!   signing a challenge, so only you can fetch (and delete) your parked messages.
//! - **Push** is open but signed by the sender, like dropping a letter in a box.
//!
//! Replay of a pull request is prevented by a client timestamp the server bounds to a
//! short window. (Metadata privacy is a later TLS/relay concern; per the threat model
//! the helper is untrusted and sees ciphertext only.)

use serde::{Deserialize, Serialize};

/// `POST /v1/push` — drop one ciphertext frame for `to` into the mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    /// Recipient identity (Ed25519 public key, hex) — the mailbox address.
    pub to: String,
    /// Opaque sealed payload (a serialized [`crate::MessageFrame`] or
    /// [`crate::KeyEnvelope`]), hex/base64. The server never inspects it.
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushResponse {
    pub id: String,
}

/// `POST /v1/pull` — fetch parked messages for `identity`. Must be signed by the
/// matching secret key. `signing_bytes` defines exactly what the signature covers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    /// The caller's identity (Ed25519 public key, hex). Messages addressed here are
    /// returned only to a caller who can sign for it.
    pub identity: String,
    /// Unix-millis timestamp; the server rejects requests outside a short window.
    pub ts: u64,
    /// Ed25519 signature over [`PullRequest::signing_bytes`], hex.
    pub signature: String,
}

impl PullRequest {
    pub fn signing_bytes(identity: &str, ts: u64) -> Vec<u8> {
        format!("seqr/pull/v1|{identity}|{ts}").into_bytes()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulledMessage {
    pub id: String,
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullResponse {
    pub messages: Vec<PulledMessage>,
}

/// `POST /v1/ack` — delete delivered messages. Same identity proof as pull.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AckRequest {
    pub identity: String,
    pub ts: u64,
    pub signature: String,
    pub ids: Vec<String>,
}

impl AckRequest {
    pub fn signing_bytes(identity: &str, ts: u64, ids: &[String]) -> Vec<u8> {
        format!("seqr/ack/v1|{identity}|{ts}|{}", ids.join(",")).into_bytes()
    }
}
