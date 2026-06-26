//! Seqr wire protocol — types shared by the desktop core and the mailbox service.
//!
//! Everything here is plain serializable data. Two concerns live in this crate:
//! 1. The shapes exchanged over the network (profiles, message frames, key envelopes).
//! 2. The **canonical signing strings** — the exact bytes a signature covers — so the
//!    signer and verifier never disagree about what was signed.

pub mod mailbox;

use serde::{Deserialize, Serialize};

/// 32-byte key/identifier, hex-encoded on the wire for readability and easy logging
/// (these are all public values).
pub type HexKey = String;

/// The unit of "adding a friend": only public data, safe to transmit by any channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileBlob {
    /// Schema version, so future changes stay backward-compatible.
    pub v: u8,
    pub display_name: String,
    /// X25519 public key (key agreement), hex.
    pub agreement_public: HexKey,
    /// Ed25519 public key (signature verification), hex.
    pub signing_public: HexKey,
    /// iroh node address used to reach this peer (opaque ticket string).
    pub node_addr: String,
}

impl ProfileBlob {
    pub const VERSION: u8 = 1;
}

/// A sealed, signed application message as it travels between peers (and through the
/// mailbox). The `ciphertext` is opaque to everyone but the conversation members.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageFrame {
    pub conversation_id: String,
    /// Key epoch this frame was sealed under; lets the receiver pick the right key
    /// and reject frames from a retired epoch.
    pub epoch: u64,
    /// Ed25519 public key of the sender, hex.
    pub sender: HexKey,
    /// Per-sender monotonic sequence number (ordering / replay detection).
    pub seq: u64,
    /// Detached Ed25519 signature over [`MessageFrame::signing_bytes`], hex (64 bytes).
    pub signature: HexKey,
    /// `nonce || ChaCha20-Poly1305 ciphertext`, base64-ish hex.
    pub ciphertext: HexKey,
}

impl MessageFrame {
    /// Canonical bytes the sender signs and the receiver verifies. Binds the
    /// ciphertext to its conversation, epoch, sender, and sequence so none can be
    /// swapped without invalidating the signature.
    pub fn signing_bytes(
        conversation_id: &str,
        epoch: u64,
        sender: &str,
        seq: u64,
        ciphertext_hex: &str,
    ) -> Vec<u8> {
        format!("seqr/frame/v1|{conversation_id}|{epoch}|{sender}|{seq}|{ciphertext_hex}")
            .into_bytes()
    }
}

/// A group key delivered to one member, sealed under the pairwise key between the
/// distributor and that member. Carried in-band over the secure channel / mailbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyEnvelope {
    pub conversation_id: String,
    pub epoch: u64,
    /// Who minted/distributed this key (Ed25519 public, hex) — used for the
    /// concurrent-rotation tiebreak (highest epoch, then lowest originator).
    pub originator: HexKey,
    /// `nonce || ciphertext` of the 32-byte group key, sealed under the pairwise key.
    pub sealed_key: HexKey,
}
