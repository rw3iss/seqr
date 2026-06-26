//! Vault key derivation from the user's login password (Argon2id).
//!
//! The login password is stretched into the 32-byte key that encrypts the local
//! SQLCipher vault. Argon2id is memory-hard, frustrating brute-force on a stolen
//! database file. The salt is generated once at account creation and stored
//! alongside the vault (a salt is not secret).

use argon2::{Algorithm, Argon2, Params, Version};
use rand_core::{OsRng, RngCore};

use crate::{CryptoError, SymmetricKey};

/// Generate a fresh random 16-byte salt for a new account.
pub fn generate_salt() -> [u8; 16] {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    salt
}

/// Derive the 32-byte vault key from `password` and `salt`.
///
/// Parameters (19 MiB, 2 passes, 1 lane) follow the OWASP Argon2id baseline —
/// strong yet quick enough for an interactive desktop login.
pub fn derive_vault_key(password: &[u8], salt: &[u8]) -> Result<SymmetricKey, CryptoError> {
    let params = Params::new(19 * 1024, 2, 1, Some(32)).map_err(|_| CryptoError::Kdf)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(password, salt, &mut key)
        .map_err(|_| CryptoError::Kdf)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_same_inputs() {
        let salt = [1u8; 16];
        let a = derive_vault_key(b"correct horse", &salt).unwrap();
        let b = derive_vault_key(b"correct horse", &salt).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn salt_changes_output() {
        let a = derive_vault_key(b"pw", &[1u8; 16]).unwrap();
        let b = derive_vault_key(b"pw", &[2u8; 16]).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn password_changes_output() {
        let salt = [9u8; 16];
        assert_ne!(
            derive_vault_key(b"alpha", &salt).unwrap(),
            derive_vault_key(b"beta", &salt).unwrap()
        );
    }
}
