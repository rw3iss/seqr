//! Profile export/import — the unit of "adding a friend".
//!
//! A profile is public-only: the X25519 and Ed25519 public keys plus the iroh node
//! address. It is encoded as `seqr:<hex>` where `<hex>` is the hex of the compact JSON
//! [`ProfileBlob`], yielding a single copy-pasteable token that survives any channel
//! intact. No secret material is ever included.

use seqr_protocol::ProfileBlob;

use super::vault::{Friend, VaultData};
use super::CoreError;

const TOKEN_PREFIX: &str = "seqr:";

/// Build this account's shareable profile from the unlocked vault.
pub fn profile_for(data: &VaultData) -> Result<ProfileBlob, CoreError> {
    let identity = data.identity()?;
    let public = identity.public();
    Ok(ProfileBlob {
        v: ProfileBlob::VERSION,
        display_name: data.display_name.clone(),
        agreement_public: hex::encode(public.agreement_public),
        signing_public: hex::encode(public.signing_public),
        node_addr: data.node_addr.clone(),
    })
}

/// Encode a profile as a `seqr:<hex>` token.
pub fn encode_token(profile: &ProfileBlob) -> Result<String, CoreError> {
    let json = serde_json::to_vec(profile).map_err(|e| CoreError::BadProfile(e.to_string()))?;
    Ok(format!("{TOKEN_PREFIX}{}", hex::encode(json)))
}

/// Decode a `seqr:<hex>` token back into a profile, validating its shape.
pub fn decode_token(token: &str) -> Result<ProfileBlob, CoreError> {
    let hex_part = token
        .trim()
        .strip_prefix(TOKEN_PREFIX)
        .ok_or_else(|| CoreError::BadProfile("missing seqr: prefix".into()))?;
    let bytes = hex::decode(hex_part).map_err(|_| CoreError::BadProfile("not valid hex".into()))?;
    let profile: ProfileBlob =
        serde_json::from_slice(&bytes).map_err(|e| CoreError::BadProfile(e.to_string()))?;
    if profile.v != ProfileBlob::VERSION {
        return Err(CoreError::BadProfile(format!("unsupported version {}", profile.v)));
    }
    // Validate key lengths so a malformed friend can never enter the roster.
    if hex::decode(&profile.agreement_public).map(|b| b.len()) != Ok(32)
        || hex::decode(&profile.signing_public).map(|b| b.len()) != Ok(32)
    {
        return Err(CoreError::BadProfile("public keys must be 32 bytes".into()));
    }
    Ok(profile)
}

/// Convert a decoded profile into a roster [`Friend`].
pub fn friend_from(profile: &ProfileBlob) -> Friend {
    Friend {
        display_name: profile.display_name.clone(),
        agreement_public: profile.agreement_public.clone(),
        signing_public: profile.signing_public.clone(),
        node_addr: profile.node_addr.clone(),
    }
}

/// Build a signed friend request carrying this account's public profile.
pub fn signed_friend_request(data: &VaultData) -> Result<super::packet::FriendRequest, CoreError> {
    let me = data.identity()?;
    let profile = profile_for(data)?;
    let bytes = super::packet::friend_req_signing_bytes(&profile);
    let signature = hex::encode(seqr_crypto::sign::sign(&me.signing_key, &bytes));
    Ok(super::packet::FriendRequest { profile, signature })
}

/// This account represented as a roster [`Friend`] (for inclusion in group rosters).
pub fn self_as_friend(data: &VaultData) -> Result<Friend, CoreError> {
    let p = data.identity()?.public();
    Ok(Friend {
        display_name: data.display_name.clone(),
        agreement_public: hex::encode(p.agreement_public),
        signing_public: hex::encode(p.signing_public),
        node_addr: data.node_addr.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::vault;

    #[test]
    fn export_import_roundtrip() {
        let dir = {
            let mut p = std::env::temp_dir();
            p.push(format!("seqr-id-test-{}", hex::encode(seqr_crypto::group::generate_group_key())));
            p
        };
        let (_k, data) = vault::create(&dir, "Alice", "pw").unwrap();
        let profile = profile_for(&data).unwrap();
        let token = encode_token(&profile).unwrap();
        assert!(token.starts_with("seqr:"));
        let decoded = decode_token(&token).unwrap();
        assert_eq!(decoded, profile);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_garbage_token() {
        assert!(decode_token("not-a-token").is_err());
        assert!(decode_token("seqr:zzzz").is_err());
    }
}
