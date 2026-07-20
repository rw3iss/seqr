//! Client construction and session persistence for the Matrix backend.
//!
//! Mirrors the SDK's `persist_session` example: a `FullSession` bundles the info needed
//! to rebuild the client (`ClientSession`) with the SDK's own `MatrixSession` (access
//! token + device id). On login we write it; on restart we read it and `restore_session`.
//!
//! ⚠️ TODO(security): the session file currently stores the store passphrase and access
//! token in plaintext under the app-data dir (same exposure as the old vault file). A
//! follow-up should encrypt it at rest with a key derived from the login password
//! (Argon2id), reusing `seqr-crypto`, so a stolen data dir alone can't unlock the store.

use std::path::PathBuf;

use matrix_sdk::{authentication::matrix::MatrixSession, Client};
use serde::{Deserialize, Serialize};

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
    Client::builder()
        .homeserver_url(homeserver)
        .sqlite_store(db_path, Some(passphrase))
        .build()
        .await
}

/// A fresh 32-byte hex passphrase for the SQLite stores.
pub fn new_passphrase() -> String {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("system RNG");
    hex::encode(bytes)
}
