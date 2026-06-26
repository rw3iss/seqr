//! Encrypted-at-rest local store.
//!
//! The vault is a single JSON file: a cleartext header (version + Argon2 salt) wrapping
//! a ChaCha20-Poly1305-sealed blob of all private state — identity secret keys, the
//! friends roster, and (in later milestones) conversations and message history. The
//! sealing key is derived from the login password via Argon2id; without it the file is
//! indistinguishable from noise.
//!
//! This replaces the spec's SQLCipher choice with a pure-Rust encrypted file — simpler,
//! no native dependency, and ample at friend scale. The data model can migrate to a
//! database later without changing the IPC surface.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use seqr_crypto::{aead, kdf, keys::Identity, SymmetricKey};

use super::CoreError;

const VAULT_AAD: &[u8] = b"seqr-vault-v1";

/// The decrypted contents of the vault.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VaultData {
    pub display_name: String,
    /// Hex of the X25519 secret (key agreement).
    pub agreement_secret: String,
    /// Hex of the Ed25519 secret (signing).
    pub signing_secret: String,
    /// iroh endpoint id (z-base-32); populated once the transport starts.
    pub node_addr: String,
    pub friends: Vec<Friend>,
    /// Chat history across all conversations. New field — `serde(default)` keeps older
    /// vault files loadable.
    #[serde(default)]
    pub messages: Vec<StoredMessage>,
    /// Group conversations this account is a member of.
    #[serde(default)]
    pub groups: Vec<Group>,
}

/// A group conversation. `members` lists every *other* member (this account is
/// implicit). The group is sealed with a single shared symmetric `key` at `epoch`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub members: Vec<Friend>,
    pub epoch: u64,
    /// Current group key (Kg), hex.
    pub key: String,
}

/// One stored message in a conversation's history (plaintext at rest within the
/// already-encrypted vault).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredMessage {
    pub conversation_id: String,
    /// Sender's Ed25519 public key, hex (equals our own for outgoing messages).
    pub sender: String,
    pub body: String,
    /// Unix-millis timestamp.
    pub ts: u64,
    pub outgoing: bool,
    /// Per-sender sequence number — used to deduplicate (a message may arrive both
    /// directly and via the mailbox).
    #[serde(default)]
    pub seq: u64,
}

impl VaultData {
    /// Append a message and return how many outgoing messages now exist in its
    /// conversation (useful as the next sequence number).
    pub fn add_message(&mut self, msg: StoredMessage) {
        self.messages.push(msg);
    }

    /// Messages for one conversation, in stored (chronological) order.
    pub fn history(&self, conversation_id: &str) -> Vec<StoredMessage> {
        self.messages
            .iter()
            .filter(|m| m.conversation_id == conversation_id)
            .cloned()
            .collect()
    }

    /// Count of outgoing messages in a conversation — the next outgoing seq number.
    pub fn next_seq(&self, conversation_id: &str) -> u64 {
        self.messages
            .iter()
            .filter(|m| m.conversation_id == conversation_id && m.outgoing)
            .count() as u64
    }

    /// Look up a friend by their Ed25519 (signing) public key.
    pub fn friend_by_signing(&self, signing_public: &str) -> Option<&Friend> {
        self.friends.iter().find(|f| f.signing_public == signing_public)
    }

    pub fn group_by_id(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_by_id_mut(&mut self, id: &str) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    /// The group's symmetric key as raw bytes, for the current epoch.
    pub fn group_key(&self, id: &str) -> Option<[u8; 32]> {
        self.group_by_id(id)
            .and_then(|g| hex::decode(&g.key).ok())
            .and_then(|v| v.try_into().ok())
    }

    /// True if an incoming message with this (conversation, sender, seq) is already
    /// stored — guards against the same message arriving twice (direct + mailbox).
    pub fn has_incoming(&self, conversation_id: &str, sender: &str, seq: u64) -> bool {
        self.messages.iter().any(|m| {
            !m.outgoing
                && m.conversation_id == conversation_id
                && m.sender == sender
                && m.seq == seq
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Friend {
    pub display_name: String,
    pub agreement_public: String,
    pub signing_public: String,
    pub node_addr: String,
}

/// The on-disk envelope. Header fields are public; `sealed` is the encrypted [`VaultData`].
#[derive(Debug, Serialize, Deserialize)]
struct VaultFile {
    v: u8,
    salt: String,
    sealed: String,
}

impl VaultData {
    /// Reconstruct the live [`Identity`] from the stored secret bytes.
    pub fn identity(&self) -> Result<Identity, CoreError> {
        let agreement = decode32(&self.agreement_secret)?;
        let signing = decode32(&self.signing_secret)?;
        Identity::from_secret_bytes(&agreement, &signing).map_err(Into::into)
    }
}

fn decode32(hex_str: &str) -> Result<[u8; 32], CoreError> {
    let bytes = hex::decode(hex_str).map_err(|_| CoreError::Storage("bad hex".into()))?;
    bytes.try_into().map_err(|_| CoreError::Storage("bad key length".into()))
}

pub fn vault_path(data_dir: &Path) -> PathBuf {
    data_dir.join("vault.json")
}

pub fn exists(data_dir: &Path) -> bool {
    vault_path(data_dir).exists()
}

/// Create a brand-new vault for a fresh account. Returns the derived vault key so the
/// caller can keep the account unlocked. Fails if a vault already exists.
pub fn create(data_dir: &Path, display_name: &str, password: &str) -> Result<(SymmetricKey, VaultData), CoreError> {
    if exists(data_dir) {
        return Err(CoreError::AccountExists);
    }
    std::fs::create_dir_all(data_dir)?;

    let identity = Identity::generate();
    let (agreement, signing) = identity.secret_bytes();
    let data = VaultData {
        display_name: display_name.to_string(),
        agreement_secret: hex::encode(agreement),
        signing_secret: hex::encode(signing),
        node_addr: String::new(),
        friends: Vec::new(),
        messages: Vec::new(),
        groups: Vec::new(),
    };

    let salt = kdf::generate_salt();
    let key = kdf::derive_vault_key(password.as_bytes(), &salt)?;
    write_sealed(data_dir, &salt, &key, &data)?;
    Ok((key, data))
}

/// Unlock an existing vault with the password. Returns the vault key and decrypted data.
pub fn unlock(data_dir: &Path, password: &str) -> Result<(SymmetricKey, VaultData), CoreError> {
    let raw = std::fs::read_to_string(vault_path(data_dir)).map_err(|_| CoreError::NoAccount)?;
    let file: VaultFile = serde_json::from_str(&raw).map_err(|e| CoreError::Storage(e.to_string()))?;
    let salt = hex::decode(&file.salt).map_err(|_| CoreError::Storage("bad salt".into()))?;
    let key = kdf::derive_vault_key(password.as_bytes(), &salt)?;
    let sealed = hex::decode(&file.sealed).map_err(|_| CoreError::Storage("bad sealed".into()))?;
    // A decryption failure here means the wrong password (key) was supplied.
    let plain = aead::open(&key, &sealed, VAULT_AAD).map_err(|_| CoreError::BadPassword)?;
    let data: VaultData = serde_json::from_slice(&plain).map_err(|e| CoreError::Storage(e.to_string()))?;
    Ok((key, data))
}

/// Persist updated data, re-using the established salt and key.
pub fn save(data_dir: &Path, key: &SymmetricKey, data: &VaultData) -> Result<(), CoreError> {
    let raw = std::fs::read_to_string(vault_path(data_dir)).map_err(|_| CoreError::NoAccount)?;
    let file: VaultFile = serde_json::from_str(&raw).map_err(|e| CoreError::Storage(e.to_string()))?;
    let salt = hex::decode(&file.salt).map_err(|_| CoreError::Storage("bad salt".into()))?;
    write_sealed(data_dir, &salt, key, data)
}

fn write_sealed(data_dir: &Path, salt: &[u8], key: &SymmetricKey, data: &VaultData) -> Result<(), CoreError> {
    let plain = serde_json::to_vec(data).map_err(|e| CoreError::Storage(e.to_string()))?;
    let sealed = aead::seal(key, &plain, VAULT_AAD);
    let file = VaultFile { v: 1, salt: hex::encode(salt), sealed: hex::encode(sealed) };
    let text = serde_json::to_string_pretty(&file).map_err(|e| CoreError::Storage(e.to_string()))?;
    // Write atomically: temp file then rename, so a crash never truncates the vault.
    let tmp = vault_path(data_dir).with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, vault_path(data_dir))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("seqr-vault-test-{}", hex::encode(seqr_crypto::group::generate_group_key())));
        p
    }

    #[test]
    fn create_unlock_roundtrip() {
        let dir = tmp_dir();
        let (_k, data) = create(&dir, "Alice", "hunter2").unwrap();
        assert_eq!(data.display_name, "Alice");

        let (_k2, reopened) = unlock(&dir, "hunter2").unwrap();
        assert_eq!(reopened.agreement_secret, data.agreement_secret);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn wrong_password_rejected() {
        let dir = tmp_dir();
        create(&dir, "Alice", "correct").unwrap();
        assert!(matches!(unlock(&dir, "wrong"), Err(CoreError::BadPassword)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_persists_friends() {
        let dir = tmp_dir();
        let (key, mut data) = create(&dir, "Alice", "pw").unwrap();
        data.friends.push(Friend {
            display_name: "Bob".into(),
            agreement_public: "aa".into(),
            signing_public: "bb".into(),
            node_addr: "node".into(),
        });
        save(&dir, &key, &data).unwrap();
        let (_k, reopened) = unlock(&dir, "pw").unwrap();
        assert_eq!(reopened.friends.len(), 1);
        assert_eq!(reopened.friends[0].display_name, "Bob");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn double_create_fails() {
        let dir = tmp_dir();
        create(&dir, "A", "pw").unwrap();
        assert!(matches!(create(&dir, "A", "pw"), Err(CoreError::AccountExists)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
