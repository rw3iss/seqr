//! Client construction and session persistence for the Matrix backend.
//!
//! Mirrors the SDK's `persist_session` example: a `FullSession` bundles the info needed
//! to rebuild the client (`ClientSession`) with the SDK's own `MatrixSession` (access
//! token + device id). On login we write it; on restart we read it and `restore_session`.
//!
//! The session file is **encrypted at rest**: a key is derived from the login password via
//! Argon2id (`seqr-crypto`) and used to AEAD-seal the `FullSession` (which holds the access
//! token + store passphrase). A stolen data dir alone therefore can't unlock the store; the
//! password is required at launch (see `matrix_unlock`).

use std::path::Path;
use std::path::PathBuf;

use base64::Engine;
use matrix_sdk::{authentication::matrix::MatrixSession, Client};
use serde::{Deserialize, Serialize};
use seqr_crypto::{aead, kdf};

/// Everything needed to rebuild the `Client` (independent of the SDK's session type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSession {
    /// Homeserver base URL the client was built against.
    pub homeserver: String,
    /// Directory backing the SQLite stores.
    pub db_path: PathBuf,
    /// Passphrase encrypting the SQLite stores at rest.
    pub passphrase: String,
}

/// Persisted on disk between runs: client rebuild info + the SDK session + a sync cursor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullSession {
    pub client_session: ClientSession,
    pub user_session: MatrixSession,
    /// Opaque sync token to resume `/sync` where we left off (set after first sync).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync_token: Option<String>,
}

/// Build a persistent client against `homeserver`, backing its stores at `db_path`
/// (encrypted with `passphrase`).
pub async fn build_client(
    homeserver: &str,
    db_path: &PathBuf,
    passphrase: &str,
) -> Result<Client, matrix_sdk::ClientBuildError> {
    use matrix_sdk::config::RequestConfig;
    use std::time::Duration;
    eprintln!("[seqr] build_client: start (homeserver={homeserver})");
    // Bound every request so a stalled connection surfaces as an error instead of an
    // infinite "Working…" hang. retry_timeout caps the whole retry window.
    let req = RequestConfig::new()
        .timeout(Duration::from_secs(30))
        .disable_retry();
    let r = Client::builder()
        .homeserver_url(homeserver)
        .request_config(req)
        .sqlite_store(db_path, Some(passphrase))
        .build()
        .await;
    eprintln!("[seqr] build_client: done ok={}", r.is_ok());
    r
}

/// A fresh 32-byte hex passphrase for the SQLite stores.
pub fn new_passphrase() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("system RNG");
    hex::encode(bytes)
}

/// On-disk envelope for the encrypted session: cleartext Argon2 salt wrapping the sealed
/// `FullSession`.
#[derive(Serialize, Deserialize)]
struct SealedSession {
    salt: String,
    sealed: String,
}

const SESSION_AAD: &[u8] = b"seqr-matrix-session-v1";

/// Encrypt + write the session under a key derived from `password` (Argon2id).
pub fn write_encrypted_session(
    path: &Path,
    password: &str,
    full: &FullSession,
) -> Result<(), String> {
    let salt = kdf::generate_salt();
    let key = kdf::derive_vault_key(password.as_bytes(), &salt).map_err(|e| e.to_string())?;
    let plaintext = serde_json::to_vec(full).map_err(|e| e.to_string())?;
    let sealed = aead::seal(&key, &plaintext, SESSION_AAD);
    let b64 = base64::engine::general_purpose::STANDARD;
    let env = SealedSession {
        salt: b64.encode(salt),
        sealed: b64.encode(sealed),
    };
    let json = serde_json::to_string(&env).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Read + decrypt the session with `password`. Errors (incl. a wrong password) surface as
/// a generic message.
pub fn read_encrypted_session(path: &Path, password: &str) -> Result<FullSession, String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let env: SealedSession = serde_json::from_str(&json).map_err(|e| e.to_string())?;
    let b64 = base64::engine::general_purpose::STANDARD;
    let salt = b64.decode(&env.salt).map_err(|e| e.to_string())?;
    let sealed = b64.decode(&env.sealed).map_err(|e| e.to_string())?;
    let key = kdf::derive_vault_key(password.as_bytes(), &salt).map_err(|e| e.to_string())?;
    let plaintext = aead::open(&key, &sealed, SESSION_AAD).map_err(|_| "wrong password".to_string())?;
    serde_json::from_slice(&plaintext).map_err(|e| e.to_string())
}
