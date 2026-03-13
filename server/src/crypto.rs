//! AES-256-GCM encryption — wire-compatible with the client-side Web Crypto
//! implementation in `web/src/crypto/crypto.ts`.
//!
//! Wire format: `[12-byte random nonce][AES-GCM ciphertext + 16-byte auth tag]`

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};

/// Nonce length used by AES-256-GCM (96 bits).
const NONCE_LENGTH: usize = 12;

/// Encrypt `plaintext` with AES-256-GCM using the given 32-byte `key`.
/// Returns `[12-byte nonce][ciphertext + 16-byte auth tag]`.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| format!("Invalid AES key: {}", e))?;

    let nonce_bytes = aes_gcm::aead::rand_core::RngCore::next_u64(&mut OsRng);
    let mut nonce_arr = [0u8; NONCE_LENGTH];
    // AES-GCM requires a 96-bit (12-byte) nonce. We fill it from two random u64
    // values: 8 bytes from the first call, 4 bytes from the second.
    let extra = aes_gcm::aead::rand_core::RngCore::next_u64(&mut OsRng);
    nonce_arr[..8].copy_from_slice(&nonce_bytes.to_le_bytes());
    nonce_arr[8..].copy_from_slice(&extra.to_le_bytes()[..4]);
    let nonce = Nonce::from_slice(&nonce_arr);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("Encryption failed: {}", e))?;

    let mut result = Vec::with_capacity(NONCE_LENGTH + ciphertext.len());
    result.extend_from_slice(&nonce_arr);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Decrypt data produced by [`encrypt`] (or the equivalent client-side code).
/// Input format: `[12-byte nonce][ciphertext + 16-byte auth tag]`.
///
/// Currently used by the background converter during encrypted→web-format
/// migration. May appear unused in some build configurations.
#[allow(dead_code)]
pub fn decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, String> {
    // Minimum valid ciphertext: 12-byte nonce + 16-byte GCM authentication tag
    if data.len() < NONCE_LENGTH + 16 {
        return Err("Ciphertext too short".into());
    }
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| format!("Invalid AES key: {}", e))?;
    let nonce = Nonce::from_slice(&data[..NONCE_LENGTH]);
    let ciphertext = &data[NONCE_LENGTH..];
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {}", e))
}

/// Parse a hex-encoded AES-256 key (64 hex chars → 32 bytes).
///
/// Used by both the encryption migration endpoint
/// (`POST /api/admin/encryption/migrate`) and the reconvert pipeline.
pub fn parse_key_hex(hex_str: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid hex key: {}", e))?;
    if bytes.len() != 32 {
        return Err(format!("Key must be 32 bytes, got {}", bytes.len()));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = [0x42u8; 32];
        let plaintext = b"Hello, simple-photos!";
        let encrypted = encrypt(&key, plaintext).unwrap();
        assert!(encrypted.len() > plaintext.len());
        let decrypted = decrypt(&key, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn parse_hex_key() {
        let hex = "aa".repeat(32);
        let key = parse_key_hex(&hex).unwrap();
        assert_eq!(key, [0xaa; 32]);
    }
}
