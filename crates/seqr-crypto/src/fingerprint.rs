//! Safety numbers — a human-comparable fingerprint of two identities.
//!
//! Two friends can read this number aloud (or compare by another trusted channel) to
//! confirm no one substituted a public key during the profile exchange — the same idea
//! as Signal's safety numbers. It is derived from both signing public keys in a
//! canonical (order-independent) way, so both sides compute the identical value.

use sha2::{Digest, Sha256};

const DOMAIN: &[u8] = b"seqr-safety-v1";

/// Compute the safety number for two Ed25519 signing public keys, as six space-
/// separated five-digit groups (e.g. "01234 56789 …").
pub fn safety_number(a: &[u8; 32], b: &[u8; 32]) -> String {
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(lo);
    hasher.update(hi);
    let digest = hasher.finalize();

    let mut groups = Vec::with_capacity(6);
    for i in 0..6 {
        let chunk: [u8; 4] = digest[i * 4..i * 4 + 4].try_into().expect("4-byte chunk");
        groups.push(format!("{:05}", u32::from_be_bytes(chunk) % 100_000));
    }
    groups.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_independent() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        assert_eq!(safety_number(&a, &b), safety_number(&b, &a));
    }

    #[test]
    fn distinct_pairs_differ() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let c = [3u8; 32];
        assert_ne!(safety_number(&a, &b), safety_number(&a, &c));
    }

    #[test]
    fn format_is_six_five_digit_groups() {
        let s = safety_number(&[9u8; 32], &[7u8; 32]);
        let groups: Vec<&str> = s.split(' ').collect();
        assert_eq!(groups.len(), 6);
        assert!(groups.iter().all(|g| g.len() == 5 && g.chars().all(|c| c.is_ascii_digit())));
    }
}
