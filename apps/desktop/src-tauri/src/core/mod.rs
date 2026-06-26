//! Seqr desktop core.
//!
//! All secrets and persistence live here, behind the Tauri IPC boundary — the Preact
//! UI never sees a private key or plaintext history. Modules:
//! - [`config`]   — runtime settings (the VPS mailbox endpoint).
//! - [`vault`]    — the encrypted-at-rest local store (identity, friends, messages).
//! - [`identity`] — profile export/import (public-only blobs).
//! - [`session`]  — in-memory state of the currently unlocked account.
//!
//! Milestone status: vault + identity + friends roster are implemented. Live
//! transport (iroh), the mailbox client, group key distribution, and message
//! send/receive are scaffolded in the spec and land in later milestones.

pub mod config;
pub mod identity;
pub mod session;
pub mod vault;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("no account exists yet")]
    NoAccount,
    #[error("an account already exists")]
    AccountExists,
    #[error("incorrect password")]
    BadPassword,
    #[error("the account is locked")]
    Locked,
    #[error("invalid profile blob: {0}")]
    BadProfile(String),
    #[error("this friend is already in your roster")]
    DuplicateFriend,
    #[error("storage error: {0}")]
    Storage(String),
    #[error("crypto error: {0}")]
    Crypto(String),
}

impl From<seqr_crypto::CryptoError> for CoreError {
    fn from(e: seqr_crypto::CryptoError) -> Self {
        CoreError::Crypto(e.to_string())
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::Storage(e.to_string())
    }
}

// Tauri commands return `Result<T, String>`; surface a friendly message to the UI.
impl serde::Serialize for CoreError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

pub type CoreResult<T> = Result<T, CoreError>;
