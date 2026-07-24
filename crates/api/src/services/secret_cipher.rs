//! AES-256-GCM encryption for tenant secrets stored in the database.
//!
//! Stored format: `enc:v1:<base64(nonce || ciphertext)>` with a random
//! 12-byte nonce per encryption. Values without the `enc:v1:` prefix are
//! legacy plaintext and pass through `decrypt` unchanged (lazy migration —
//! they are re-encrypted on the next settings save).
//!
//! The key comes from `SETTINGS_ENCRYPTION_KEY` (base64-encoded 32 bytes).
//! When unset: development stores plaintext (with a one-time warning);
//! production refuses to store secrets at all.

use aes_gcm::aead::{Aead, OsRng, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;

const ENC_PREFIX: &str = "enc:v1:";
const NONCE_LEN: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum SecretCipherError {
    #[error("SETTINGS_ENCRYPTION_KEY is not configured — refusing to handle tenant secrets")]
    KeyMissing,
    #[error("SETTINGS_ENCRYPTION_KEY must be base64-encoded 32 bytes")]
    InvalidKey,
    #[error("stored secret failed to decrypt (wrong key or tampered value)")]
    DecryptFailed,
}

/// Encrypts/decrypts tenant secrets. Cheap to construct, no interior state.
pub struct SecretCipher {
    cipher: Option<Aes256Gcm>,
    /// Development only: allow storing plaintext when no key is configured.
    allow_plaintext: bool,
}

impl SecretCipher {
    /// Build from the base64-encoded 32-byte key. `None`/empty key is allowed;
    /// whether operations then succeed depends on `allow_plaintext` (dev mode).
    pub fn new(key_b64: Option<&str>, allow_plaintext: bool) -> Result<Self, SecretCipherError> {
        let cipher = match key_b64.filter(|s| !s.is_empty()) {
            Some(b64) => {
                let bytes = B64.decode(b64).map_err(|_| SecretCipherError::InvalidKey)?;
                if bytes.len() != 32 {
                    return Err(SecretCipherError::InvalidKey);
                }
                Some(Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&bytes)))
            }
            None => None,
        };
        Ok(Self {
            cipher,
            allow_plaintext,
        })
    }

    /// Encrypt a secret for storage. Without a key: development passes the
    /// plaintext through (warning once), production returns an error.
    pub fn encrypt(&self, plaintext: &str) -> Result<String, SecretCipherError> {
        let Some(cipher) = &self.cipher else {
            if self.allow_plaintext {
                warn_plaintext_once();
                return Ok(plaintext.to_string());
            }
            return Err(SecretCipherError::KeyMissing);
        };

        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|_| SecretCipherError::DecryptFailed)?;

        let mut payload = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        payload.extend_from_slice(&nonce_bytes);
        payload.extend_from_slice(&ciphertext);
        Ok(format!("{ENC_PREFIX}{}", B64.encode(payload)))
    }

    /// Decrypt a stored value. Legacy plaintext (no `enc:v1:` prefix) passes
    /// through unchanged; encrypted values require the key.
    pub fn decrypt(&self, stored: &str) -> Result<String, SecretCipherError> {
        let Some(b64) = stored.strip_prefix(ENC_PREFIX) else {
            return Ok(stored.to_string());
        };
        let Some(cipher) = &self.cipher else {
            return Err(SecretCipherError::KeyMissing);
        };

        let payload = B64
            .decode(b64)
            .map_err(|_| SecretCipherError::DecryptFailed)?;
        if payload.len() <= NONCE_LEN {
            return Err(SecretCipherError::DecryptFailed);
        }
        let (nonce_bytes, ciphertext) = payload.split_at(NONCE_LEN);
        let plaintext = cipher
            .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
            .map_err(|_| SecretCipherError::DecryptFailed)?;
        String::from_utf8(plaintext).map_err(|_| SecretCipherError::DecryptFailed)
    }
}

fn warn_plaintext_once() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        tracing::warn!(
            "SETTINGS_ENCRYPTION_KEY not set — tenant API keys will be stored in PLAINTEXT \
             (allowed in development only; set the key to enable encryption at rest)"
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // Base64 of bytes 0x00..0x1f. Test-only key, never used in deployments.
    const TEST_KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";

    fn cipher() -> SecretCipher {
        SecretCipher::new(Some(TEST_KEY_B64), false).unwrap()
    }

    #[test]
    fn round_trip() {
        let c = cipher();
        let stored = c.encrypt("sk-test-secret-key-1234").unwrap();
        assert!(stored.starts_with("enc:v1:"));
        assert_eq!(c.decrypt(&stored).unwrap(), "sk-test-secret-key-1234");
    }

    #[test]
    fn nonce_is_random_per_encryption() {
        let c = cipher();
        assert_ne!(c.encrypt("same").unwrap(), c.encrypt("same").unwrap());
    }

    #[test]
    fn legacy_plaintext_passes_through() {
        let c = cipher();
        assert_eq!(c.decrypt("sk-legacy-plain").unwrap(), "sk-legacy-plain");
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let c = cipher();
        let stored = c.encrypt("sk-test").unwrap();
        let mut payload = B64.decode(stored.strip_prefix("enc:v1:").unwrap()).unwrap();
        let last = payload.len() - 1;
        payload[last] ^= 0x01;
        let tampered = format!("enc:v1:{}", B64.encode(payload));
        assert!(matches!(
            c.decrypt(&tampered),
            Err(SecretCipherError::DecryptFailed)
        ));
    }

    #[test]
    fn wrong_key_fails() {
        let stored = cipher().encrypt("sk-test").unwrap();
        // Base64 of bytes 0x20..0x3f
        let other =
            SecretCipher::new(Some("ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8="), false).unwrap();
        assert!(matches!(
            other.decrypt(&stored),
            Err(SecretCipherError::DecryptFailed)
        ));
    }

    #[test]
    fn missing_key_encrypt_refused_outside_dev() {
        let c = SecretCipher::new(None, false).unwrap();
        assert!(matches!(
            c.encrypt("sk-test"),
            Err(SecretCipherError::KeyMissing)
        ));
    }

    #[test]
    fn missing_key_dev_stores_plaintext() {
        let c = SecretCipher::new(None, true).unwrap();
        assert_eq!(c.encrypt("sk-test").unwrap(), "sk-test");
    }

    #[test]
    fn missing_key_cannot_decrypt_encrypted_value() {
        let stored = cipher().encrypt("sk-test").unwrap();
        let keyless = SecretCipher::new(None, true).unwrap();
        assert!(matches!(
            keyless.decrypt(&stored),
            Err(SecretCipherError::KeyMissing)
        ));
        // but plaintext still passes through
        assert_eq!(keyless.decrypt("sk-plain").unwrap(), "sk-plain");
    }

    #[test]
    fn invalid_key_rejected() {
        assert!(matches!(
            SecretCipher::new(Some("not-base64!!"), false),
            Err(SecretCipherError::InvalidKey)
        ));
        // valid base64 but wrong length
        assert!(matches!(
            SecretCipher::new(Some("AAECAw=="), false),
            Err(SecretCipherError::InvalidKey)
        ));
    }

    // Must stay in sync with apps/workers/tests/test_secret_cipher.py —
    // proves the Rust and Python implementations share one format.
    #[test]
    fn cross_language_test_vector() {
        // Generated with the Python implementation: key = TEST_KEY_B64,
        // nonce = bytes 0..11, plaintext = "sk-test-secret-key-1234".
        let vector = "enc:v1:AAECAwQFBgcICQoLNGn7b6CWtjb+JPT51J1VBuavqgXCSGv5XIfps4HrjZSgvnC5bMoI";
        assert_eq!(cipher().decrypt(vector).unwrap(), "sk-test-secret-key-1234");
    }
}
