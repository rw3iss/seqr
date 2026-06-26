//! The top-level unit carried over the transport / mailbox.
//!
//! Everything a peer sends is a `Packet`: either a chat `Message` (a sealed, signed
//! [`MessageFrame`] for a 1:1 or a group), or a `GroupInvite` that hands a new member
//! the group roster and the group key sealed to them. Keeping one envelope means the
//! receive path has a single parse + dispatch point.

use serde::{Deserialize, Serialize};

use seqr_protocol::{MessageFrame, ProfileBlob};

use super::vault::Friend;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Packet {
    /// A chat message (1:1 or group); the conversation id distinguishes which.
    Message(MessageFrame),
    /// Invitation to (or membership refresh for) a group, addressed to one recipient.
    GroupInvite(GroupInvite),
    /// A rotated key for a 1:1 conversation, sealed under the identity pairwise key.
    KeyUpdate(KeyUpdate),
    /// "I added you — add me back?" Carries the requester's signed public profile.
    /// Accepted from *unknown* senders (that's the point) after signature verification.
    FriendRequest(FriendRequest),
    /// Header announcing an incoming attachment (followed by AttachmentChunk packets).
    AttachmentMeta(AttachmentMeta),
    /// One encrypted chunk of an attachment.
    AttachmentChunk(AttachmentChunk),
}

/// Announces an attachment: its identity, the conversation/epoch it belongs to, and how
/// many chunks to expect. Signed by the sender (binds the metadata to the identity).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentMeta {
    pub att_id: String,
    pub conversation_id: String,
    pub epoch: u64,
    pub sender: String,
    pub seq: u64,
    pub filename: String,
    pub mime: String,
    pub size: u64,
    pub chunks: u32,
    pub signature: String,
}

impl AttachmentMeta {
    pub fn signing_bytes(&self) -> Vec<u8> {
        format!(
            "seqr/attmeta/v1|{}|{}|{}|{}|{}|{}|{}|{}|{}",
            self.att_id,
            self.conversation_id,
            self.epoch,
            self.sender,
            self.seq,
            self.filename,
            self.mime,
            self.size,
            self.chunks
        )
        .into_bytes()
    }
}

/// One encrypted attachment chunk (`data` is hex of `seal_chunk` output).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentChunk {
    pub att_id: String,
    pub index: u32,
    pub data: String,
}

/// A friend request: the requester's public profile, signed by their identity key so
/// the agreement key is provably bound to the signing key (no in-transit key swap).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendRequest {
    pub profile: ProfileBlob,
    /// Ed25519 signature over [`friend_req_signing_bytes`], hex.
    pub signature: String,
}

/// Canonical bytes a friend request signs/verifies — binds all profile fields together.
pub fn friend_req_signing_bytes(p: &ProfileBlob) -> Vec<u8> {
    format!(
        "seqr/friendreq/v1|{}|{}|{}|{}",
        p.signing_public, p.agreement_public, p.node_addr, p.display_name
    )
    .into_bytes()
}

/// Carries a freshly rotated 1:1 key, sealed under the long-term pairwise key between
/// the two parties so the recipient can open it without a new profile exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyUpdate {
    pub conversation_id: String,
    pub epoch: u64,
    /// Sender's Ed25519 public key (hex) — the originator, found in the recipient's roster.
    pub originator: String,
    /// `nonce || ciphertext` of the 32-byte key, sealed under the identity pairwise key.
    pub sealed_key: String,
}

/// Sent individually to each member: carries the full roster plus the group key sealed
/// under the pairwise key between the originator and this recipient.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupInvite {
    pub group_id: String,
    pub name: String,
    pub epoch: u64,
    /// Every member of the group (public info), including the originator.
    pub members: Vec<Friend>,
    /// The originator's Ed25519 public key (hex) — who minted/sent this key.
    pub originator: String,
    /// `nonce || ciphertext` of the 32-byte group key, sealed under the pairwise key.
    pub sealed_key: String,
}

impl Packet {
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("packet serializes")
    }
}
