//! Message framing: turn plaintext into a signed, sealed [`MessageFrame`] and back.
//!
//! Pure functions over keys and bytes — no I/O, no networking — so the security-
//! critical path is unit-tested in isolation. A frame is:
//! - **sealed** with the conversation's symmetric key (ChaCha20-Poly1305), the AAD
//!   binding it to its conversation + epoch so it cannot be replayed elsewhere;
//! - **signed** with the sender's Ed25519 key, so the receiver (and, in groups, every
//!   member) can prove who authored it.

use seqr_crypto::{aead, keys::Identity, sign, SymmetricKey};
use seqr_protocol::MessageFrame;

use super::CoreError;

/// Associated data binding a ciphertext to its conversation and epoch.
fn aad(conversation_id: &str, epoch: u64) -> Vec<u8> {
    format!("{conversation_id}|{epoch}").into_bytes()
}

/// Build a signed, sealed frame from plaintext.
pub fn build_frame(
    identity: &Identity,
    conversation_id: &str,
    epoch: u64,
    key: &SymmetricKey,
    seq: u64,
    plaintext: &str,
) -> MessageFrame {
    let sender = hex::encode(identity.public().signing_public);
    let ciphertext = aead::seal(key, plaintext.as_bytes(), &aad(conversation_id, epoch));
    let ciphertext_hex = hex::encode(ciphertext);
    let signing_bytes =
        MessageFrame::signing_bytes(conversation_id, epoch, &sender, seq, &ciphertext_hex);
    let signature = hex::encode(sign::sign(&identity.signing_key, &signing_bytes));
    MessageFrame {
        conversation_id: conversation_id.to_string(),
        epoch,
        sender,
        seq,
        signature,
        ciphertext: ciphertext_hex,
    }
}

/// Verify a frame's signature and open its ciphertext. Returns the plaintext.
///
/// `key` must be the conversation key for this `frame.epoch`. The sender's public key
/// is taken from the frame and the signature is checked against it; the caller is
/// responsible for confirming that sender is an expected member of the conversation.
pub fn open_frame(key: &SymmetricKey, frame: &MessageFrame) -> Result<String, CoreError> {
    let sender = decode32(&frame.sender)?;
    let sig: [u8; 64] = hex::decode(&frame.signature)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(CoreError::BadProfile("bad signature".into()))?;
    let signing_bytes = MessageFrame::signing_bytes(
        &frame.conversation_id,
        frame.epoch,
        &frame.sender,
        frame.seq,
        &frame.ciphertext,
    );
    sign::verify_raw(&sender, &signing_bytes, &sig)?;

    let ciphertext = hex::decode(&frame.ciphertext)
        .map_err(|_| CoreError::Crypto("bad ciphertext hex".into()))?;
    let plain = aead::open(key, &ciphertext, &aad(&frame.conversation_id, frame.epoch))?;
    String::from_utf8(plain).map_err(|_| CoreError::Crypto("message not valid UTF-8".into()))
}

fn decode32(hex_str: &str) -> Result<[u8; 32], CoreError> {
    hex::decode(hex_str)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or(CoreError::BadProfile("bad key".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use seqr_crypto::agreement::derive_pairwise;

    #[test]
    fn frame_roundtrip_between_two_parties() {
        let alice = Identity::generate();
        let bob = Identity::generate();

        // Both derive the same pairwise key.
        let key_a = derive_pairwise(&alice.agreement_secret, &bob.public().agreement_public());
        let key_b = derive_pairwise(&bob.agreement_secret, &alice.public().agreement_public());
        assert_eq!(key_a, key_b);

        let frame = build_frame(&alice, "conv-1", 0, &key_a, 0, "hello Bob");
        let opened = open_frame(&key_b, &frame).unwrap();
        assert_eq!(opened, "hello Bob");
    }

    #[test]
    fn tampered_signature_rejected() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let key = derive_pairwise(&alice.agreement_secret, &bob.public().agreement_public());
        let mut frame = build_frame(&alice, "c", 0, &key, 0, "hi");
        frame.signature = "00".repeat(64);
        assert!(open_frame(&key, &frame).is_err());
    }

    #[test]
    fn wrong_epoch_key_rejected() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let key = derive_pairwise(&alice.agreement_secret, &bob.public().agreement_public());
        let frame = build_frame(&alice, "c", 0, &key, 0, "hi");
        // Same key but claim a different epoch via AAD mismatch.
        let mut tampered = frame.clone();
        tampered.epoch = 1;
        assert!(open_frame(&key, &tampered).is_err());
    }
}
