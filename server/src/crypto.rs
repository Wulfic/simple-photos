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
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("Invalid AES key: {}", e))?;

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
pub fn decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < NONCE_LENGTH + 16 {
        return Err("Ciphertext too short".into());
    }
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("Invalid AES key: {}", e))?;
    let nonce = Nonce::from_slice(&data[..NONCE_LENGTH]);
    let ciphertext = &data[NONCE_LENGTH..];
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {}", e))
}

/// Parse a hex-encoded AES-256 key (64 hex chars → 32 bytes).
pub fn parse_key_hex(hex_str: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(hex_str).map_err(|e| format!("Invalid hex key: {}", e))?;
    if bytes.len() != 32 {
        return Err(format!("Key must be 32 bytes, got {}", bytes.len()));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

// ── Key wrapping / unwrapping ───────────────────────────────────────────────
//
// The user's AES-256 encryption key is persisted in the DB wrapped (encrypted)
// with a key derived from the JWT secret via SHA-256, so the stored blob is
// useless without access to the server config.

use sha2::{Digest, Sha256};

/// Derive a 32-byte wrapping key from the JWT secret.
fn derive_wrapping_key(jwt_secret: &str) -> [u8; 32] {
    let hash = Sha256::digest(format!("simple-photos-key-wrap:{}", jwt_secret).as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&hash);
    key
}

/// Wrap (encrypt) a 32-byte encryption key for safe DB storage.
/// Returns a hex-encoded blob: `[12-byte nonce][ciphertext + 16-byte tag]`.
pub fn wrap_key(encryption_key: &[u8; 32], jwt_secret: &str) -> Result<String, String> {
    let wrapping_key = derive_wrapping_key(jwt_secret);
    let ciphertext = encrypt(&wrapping_key, encryption_key)?;
    Ok(hex::encode(ciphertext))
}

/// Persist the wrapped encryption key to the database.
pub async fn store_wrapped_key(
    pool: &sqlx::SqlitePool,
    encryption_key: &[u8; 32],
    jwt_secret: &str,
) -> Result<(), String> {
    let wrapped = wrap_key(encryption_key, jwt_secret)?;
    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('encryption_key_wrapped', ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(&wrapped)
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to store wrapped key: {}", e))?;

    sqlx::query(
        "INSERT INTO server_settings (key, value) VALUES ('encryption_key_active', 'true') \
         ON CONFLICT(key) DO UPDATE SET value = 'true'",
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to set key active flag: {}", e))?;

    tracing::info!("[CRYPTO] Encryption key wrapped and stored in DB");
    Ok(())
}

/// Unwrap (decrypt) a previously wrapped encryption key.
/// Input: hex-encoded blob from [`wrap_key`].
pub fn unwrap_key(wrapped_hex: &str, jwt_secret: &str) -> Result<[u8; 32], String> {
    let wrapped = hex::decode(wrapped_hex).map_err(|e| format!("Invalid hex: {}", e))?;
    let wrapping_key = derive_wrapping_key(jwt_secret);
    let plaintext = decrypt(&wrapping_key, &wrapped)?;
    if plaintext.len() != 32 {
        return Err(format!(
            "Unwrapped key is {} bytes, expected 32",
            plaintext.len()
        ));
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&plaintext);
    Ok(key)
}

/// Load the wrapped encryption key from the database (if active).
pub async fn load_wrapped_key(
    pool: &sqlx::SqlitePool,
    jwt_secret: &str,
) -> Result<Option<[u8; 32]>, String> {
    let active: Option<String> = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_key_active'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to query key active flag: {}", e))?;

    if active.as_deref() != Some("true") {
        return Ok(None);
    }

    let wrapped: Option<String> = sqlx::query_scalar(
        "SELECT value FROM server_settings WHERE key = 'encryption_key_wrapped'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("Failed to query wrapped key: {}", e))?;

    match wrapped {
        Some(hex_blob) => {
            let key = unwrap_key(&hex_blob, jwt_secret)?;
            tracing::info!("[CRYPTO] Encryption key loaded and unwrapped from DB");
            Ok(Some(key))
        }
        None => Ok(None),
    }
}

/// Clear the stored encryption key from the database.
pub async fn clear_wrapped_key(pool: &sqlx::SqlitePool) -> Result<(), String> {
    sqlx::query(
        "DELETE FROM server_settings WHERE key IN ('encryption_key_wrapped', 'encryption_key_active')",
    )
    .execute(pool)
    .await
    .map_err(|e| format!("Failed to clear wrapped key: {}", e))?;
    tracing::info!("[CRYPTO] Encryption key cleared from DB");
    Ok(())
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
