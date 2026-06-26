//! In-memory state of the currently unlocked account.
//!
//! Holds the derived vault key and decrypted [`VaultData`] only while the app is
//! unlocked; locking clears it. Guarded by a mutex and shared as Tauri managed state.

use std::path::PathBuf;
use std::sync::Mutex;

use seqr_crypto::SymmetricKey;

use super::transport::Transport;
use super::vault::VaultData;
use super::{CoreError, CoreResult};

/// The decrypted, in-use account.
pub struct Unlocked {
    pub vault_key: SymmetricKey,
    pub data: VaultData,
}

/// Shared application state. `data_dir` is fixed at startup; `unlocked` and `transport`
/// toggle with login/lock.
pub struct SessionState {
    pub data_dir: PathBuf,
    pub unlocked: Mutex<Option<Unlocked>>,
    pub transport: Mutex<Option<Transport>>,
}

impl SessionState {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { data_dir, unlocked: Mutex::new(None), transport: Mutex::new(None) }
    }

    pub fn set_transport(&self, transport: Transport) {
        *self.transport.lock().expect("transport mutex") = Some(transport);
    }

    /// Clone of the live transport, if started.
    pub fn transport(&self) -> Option<Transport> {
        self.transport.lock().expect("transport mutex").clone()
    }

    /// Run a closure against the unlocked account, erroring if locked.
    pub fn with_unlocked<T>(&self, f: impl FnOnce(&mut Unlocked) -> CoreResult<T>) -> CoreResult<T> {
        let mut guard = self.unlocked.lock().expect("session mutex");
        let unlocked = guard.as_mut().ok_or(CoreError::Locked)?;
        f(unlocked)
    }

    pub fn set_unlocked(&self, key: SymmetricKey, data: VaultData) {
        *self.unlocked.lock().expect("session mutex") = Some(Unlocked { vault_key: key, data });
    }

    pub fn lock(&self) {
        *self.unlocked.lock().expect("session mutex") = None;
        // Drop the transport handle; the endpoint shuts down once all clones are gone.
        *self.transport.lock().expect("transport mutex") = None;
    }

    pub fn is_unlocked(&self) -> bool {
        self.unlocked.lock().expect("session mutex").is_some()
    }
}
