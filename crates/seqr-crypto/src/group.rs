//! Group key generation.
//!
//! A group conversation is sealed with a single random symmetric key (`Kg`). Any
//! member may mint a new one (on rotation, or when removing a member); it is then
//! distributed to each member individually, sealed under the pairwise key
//! (see [`crate::agreement`]) — that distribution is the protocol layer's job, not
//! this crate's. Here we only produce fresh key material.

use rand_core::{OsRng, RngCore};

use crate::SymmetricKey;

/// Mint a fresh random group key from the OS CSPRNG.
pub fn generate_group_key() -> SymmetricKey {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique() {
        assert_ne!(generate_group_key(), generate_group_key());
    }
}
