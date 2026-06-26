//! Filesystem-backed message store.
//!
//! Layout: `<data_dir>/<identity_hex>/<id>.msg`, where the filename's leading
//! millisecond timestamp preserves delivery order. No database, so the binary stays
//! pure-Rust and the data is trivially inspectable/backed-up (it is only ciphertext).
//!
//! All identity/id inputs are validated to be lowercase hex before touching the
//! filesystem, which makes path traversal impossible.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use seqr_protocol::mailbox::PulledMessage;

/// Process-wide monotonic counter so messages pushed within the same millisecond
/// still sort in arrival order (the timestamp alone is not fine-grained enough).
static SEQ: AtomicU64 = AtomicU64::new(0);

pub struct Store {
    root: PathBuf,
}

/// True only for a lowercase-hex string of exactly `len` characters. Guards every
/// path component derived from client input.
pub fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn now_millis() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

impl Store {
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn mailbox_dir(&self, identity: &str) -> PathBuf {
        self.root.join(identity)
    }

    /// Park `payload` for `identity`. Returns the assigned message id.
    pub fn push(&self, identity: &str, payload: &str) -> io::Result<String> {
        let dir = self.mailbox_dir(identity);
        fs::create_dir_all(&dir)?;
        // Timestamp + monotonic counter give a strict arrival order; the random
        // suffix only guards against collisions across process restarts.
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let id = format!("{:013}-{:010}-{}", now_millis(), seq, random_suffix());
        let path = dir.join(format!("{id}.msg"));
        fs::write(&path, payload.as_bytes())?;
        Ok(id)
    }

    /// Return up to `limit` parked messages for `identity`, oldest first.
    pub fn pull(&self, identity: &str, limit: usize) -> io::Result<Vec<PulledMessage>> {
        let dir = self.mailbox_dir(identity);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries: Vec<String> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".msg"))
            .collect();
        entries.sort(); // filename sorts chronologically thanks to the timestamp prefix
        entries.truncate(limit);

        let mut out = Vec::with_capacity(entries.len());
        for name in entries {
            let payload = fs::read_to_string(dir.join(&name))?;
            let id = name.trim_end_matches(".msg").to_string();
            out.push(PulledMessage { id, payload });
        }
        Ok(out)
    }

    /// Delete the listed messages for `identity`. Ignores ids that are already gone.
    pub fn ack(&self, identity: &str, ids: &[String]) -> io::Result<()> {
        let dir = self.mailbox_dir(identity);
        for id in ids {
            // id format is `<13 digits>-<16 hex>`; reject anything else.
            if !valid_id(id) {
                continue;
            }
            let path = dir.join(format!("{id}.msg"));
            if path.exists() {
                let _ = fs::remove_file(path);
            }
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    // `<13 digits>-<10 digits>-<16 hex>`
    let parts: Vec<&str> = id.split('-').collect();
    matches!(parts.as_slice(), [ts, seq, suffix]
        if ts.len() == 13 && ts.bytes().all(|b| b.is_ascii_digit())
        && seq.len() == 10 && seq.bytes().all(|b| b.is_ascii_digit())
        && is_hex(suffix, 16))
}

fn random_suffix() -> String {
    use seqr_crypto::group::generate_group_key; // a handy CSPRNG source of bytes
    let bytes = generate_group_key();
    hex::encode(&bytes[..8])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("seqr-store-test-{}", random_suffix()));
        p
    }

    #[test]
    fn push_pull_ack_cycle() {
        let dir = tmp();
        let store = Store::new(&dir).unwrap();
        let id = "a".repeat(64);

        let m1 = store.push(&id, "ciphertext-1").unwrap();
        let _m2 = store.push(&id, "ciphertext-2").unwrap();

        let pulled = store.pull(&id, 10).unwrap();
        assert_eq!(pulled.len(), 2);
        assert_eq!(pulled[0].payload, "ciphertext-1"); // oldest first

        store.ack(&id, &[m1]).unwrap();
        let remaining = store.pull(&id, 10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].payload, "ciphertext-2");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pull_empty_mailbox() {
        let dir = tmp();
        let store = Store::new(&dir).unwrap();
        assert!(store.pull(&"b".repeat(64), 10).unwrap().is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn hex_validation() {
        assert!(is_hex(&"a".repeat(64), 64));
        assert!(!is_hex("../etc", 64));
        assert!(!is_hex("ABCDEF", 6)); // uppercase rejected
    }
}
