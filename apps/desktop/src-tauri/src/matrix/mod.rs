//! Matrix backend — the alternative to the legacy P2P `core`.
//!
//! Wraps `matrix-sdk`: builds a persistent client (SQLite state/crypto/media stores),
//! logs in against the self-hosted homeserver, and persists/restores the session so the
//! app comes back logged-in across restarts. Both backends are compiled in; which one is
//! active is chosen at launch from `AppConfig::backend`.
//!
//! Kept to a thin seam: the Tauri commands (`commands.rs`) call into here, and no
//! business logic leaks into the command layer.

pub mod client;
pub mod commands;

use std::path::PathBuf;

use matrix_sdk::Client;
use tokio::sync::RwLock;

/// Live state for the Matrix backend. Holds the client once logged in (or restored).
pub struct MatrixState {
    /// Per-user app-data dir; the Matrix stores live under `matrix/` inside it.
    data_dir: PathBuf,
    /// Homeserver base URL (from config; configurable to repoint at a new server).
    homeserver_url: String,
    /// The live client, present once logged in or restored from disk.
    client: RwLock<Option<Client>>,
}

impl MatrixState {
    pub fn new(data_dir: PathBuf, homeserver_url: String) -> Self {
        Self { data_dir, homeserver_url, client: RwLock::new(None) }
    }

    pub fn homeserver_url(&self) -> &str {
        &self.homeserver_url
    }

    /// Directory holding the SQLite stores (state, crypto, media, event cache).
    pub fn store_dir(&self) -> PathBuf {
        self.data_dir.join("matrix")
    }

    /// Encrypted session metadata (access token, device id, store passphrase).
    pub fn session_file(&self) -> PathBuf {
        self.store_dir().join("session.json")
    }
}
