//! Chunked encrypted blob format (v2) — streaming AES-GCM for large media.
//!
//! ## Why this exists
//!
//! The legacy (v1) blob format wraps the *entire* media file as base64 inside a
//! JSON envelope and encrypts it as a single AES-GCM message:
//!
//! ```text
//!   AES-GCM( {"v":1, ...metadata..., "data": base64(filebytes)} )
//! ```
//!
//! Producing that for one large video holds, simultaneously, the raw bytes
//! (1×), a base64 copy (1.33×), a serialized-JSON copy (1.33×) and the AES
//! ciphertext (1.33×) — roughly **5× the file size**, in several multi-hundred-MB
//! *contiguous* allocations. On a multi-gigabyte video that OOM-aborts the whole
//! process (`memory allocation of N bytes failed`).
//!
//! ## The v2 format
//!
//! v2 streams the file in fixed-size plaintext chunks, each encrypted as an
//! independent AES-GCM frame, so peak memory is bounded to **one chunk** (a few
//! MiB) regardless of file size — on both encrypt and decrypt.
//!
//! On-disk layout:
//!
//! ```text
//!   MAGIC                       8 bytes, [`MAGIC`]
//!   meta_len  : u32 BE          length of the encrypted metadata frame
//!   meta_frame: [meta_len]      AES-GCM frame of the metadata JSON
//!                               (same envelope as v1, MINUS "data", PLUS
//!                                "chunk_size" and "data_len")
//!   repeated until EOF:
//!     frame_len : u32 BE        length of the next encrypted chunk frame
//!     frame     : [frame_len]   AES-GCM frame of up to CHUNK_SIZE plaintext bytes
//! ```
//!
//! Every frame is *exactly* the output of [`crate::crypto::encrypt`]
//! (`[12-byte nonce][ciphertext + 16-byte tag]`), so the web and Android clients
//! decrypt each frame with the same single-frame primitive they already ship —
//! they just loop over the length-prefixed frames.

use std::io::{Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::crypto;

/// `blobs.blob_format` value for the chunked streaming format. (The legacy
/// monolithic JSON+base64 envelope is format `1`, the column default.)
pub const FORMAT_V2: i64 = 2;

/// Magic prefix for a v2 chunked blob. 8 bytes so the odds a v1 blob (which
/// begins with a random 12-byte nonce) collides with it are ~2⁻⁶⁴ — negligible,
/// which lets [`is_chunked`] auto-detect the format from the bytes alone even
/// when the caller has no `blob_format` hint.
pub const MAGIC: [u8; 8] = *b"SPCHNKB2";

/// Plaintext bytes per chunk frame (4 MiB). Each frame's ciphertext is this plus
/// the 12-byte nonce and 16-byte GCM tag. Bounds peak encrypt/decrypt memory.
pub const CHUNK_SIZE: usize = 4 * 1024 * 1024;

/// Source files at or above this size are encrypted with the chunked v2 format.
/// Smaller files keep the v1 monolithic envelope (peak ≈ 5× size is safe there,
/// and existing clients decode v1 without any changes). 32 MiB → v1 peak ≈ 160
/// MiB worst case, comfortably below the allocator ceiling on small hosts.
pub const CHUNKED_THRESHOLD_BYTES: i64 = 32 * 1024 * 1024;

/// Outcome of a streaming chunked encrypt, carrying what the caller needs for
/// the `blobs` row without ever holding the whole file in memory.
pub struct ChunkedEncryptResult {
    /// Total size of the encrypted blob written to disk.
    pub blob_size: u64,
    /// SHA-256 of the entire encrypted blob on disk (the blob `client_hash`).
    pub blob_sha256: [u8; 32],
}

/// `true` if `enc` begins with the v2 [`MAGIC`] prefix.
pub fn is_chunked(enc: &[u8]) -> bool {
    enc.len() >= MAGIC.len() && enc[..MAGIC.len()] == MAGIC
}

/// Streaming chunked encrypt. **Synchronous and blocking** — run it inside
/// `tokio::task::spawn_blocking`. Reads `src` in [`CHUNK_SIZE`] chunks,
/// encrypting each into an AES-GCM frame, and writes the v2 container to `dst`.
/// Peak heap stays at ~`CHUNK_SIZE` regardless of source size.
///
/// `metadata_json` is the photo envelope (same fields as v1 minus `data`); it is
/// encrypted as the leading metadata frame.
///
/// On any error the partially written `dst` is removed so a failed encrypt never
/// leaves a truncated blob behind.
pub fn encrypt_file_chunked(
    key: &[u8; 32],
    src: &Path,
    dst: &Path,
    metadata_json: &[u8],
) -> Result<ChunkedEncryptResult, String> {
    match encrypt_file_chunked_inner(key, src, dst, metadata_json) {
        Ok(res) => Ok(res),
        Err(e) => {
            // Clean up a half-written blob — a truncated frame is undecryptable.
            if let Err(rm) = std::fs::remove_file(dst) {
                if rm.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path = ?dst, error = %rm, "[CHUNKED] failed to clean up partial blob");
                }
            }
            Err(e)
        }
    }
}

fn encrypt_file_chunked_inner(
    key: &[u8; 32],
    src: &Path,
    dst: &Path,
    metadata_json: &[u8],
) -> Result<ChunkedEncryptResult, String> {
    let mut input = std::fs::File::open(src).map_err(|e| format!("open source: {e}"))?;
    let out_file = std::fs::File::create(dst).map_err(|e| format!("create blob: {e}"))?;
    let mut out = std::io::BufWriter::new(out_file);

    let mut blob_hasher = Sha256::new();
    let mut written: u64 = 0;

    write_tracked(&mut out, &MAGIC, &mut blob_hasher, &mut written)?;

    // Metadata frame (small) first.
    let meta_frame = crypto::encrypt(key, metadata_json)?;
    let meta_len =
        u32::try_from(meta_frame.len()).map_err(|_| "metadata frame too large".to_string())?;
    write_tracked(&mut out, &meta_len.to_be_bytes(), &mut blob_hasher, &mut written)?;
    write_tracked(&mut out, &meta_frame, &mut blob_hasher, &mut written)?;

    // Chunk frames.
    let mut buf = vec![0u8; CHUNK_SIZE];
    loop {
        let n = read_filled(&mut input, &mut buf).map_err(|e| format!("read source: {e}"))?;
        if n == 0 {
            break;
        }
        let frame = crypto::encrypt(key, &buf[..n])?;
        let frame_len =
            u32::try_from(frame.len()).map_err(|_| "chunk frame too large".to_string())?;
        write_tracked(&mut out, &frame_len.to_be_bytes(), &mut blob_hasher, &mut written)?;
        write_tracked(&mut out, &frame, &mut blob_hasher, &mut written)?;
        if n < CHUNK_SIZE {
            break; // short read ⇒ EOF; skip the extra zero-length read
        }
    }

    out.flush().map_err(|e| format!("flush blob: {e}"))?;

    Ok(ChunkedEncryptResult {
        blob_size: written,
        blob_sha256: blob_hasher.finalize().into(),
    })
}

/// Decrypt a photo blob in **either** format and return the raw media bytes.
///
/// Auto-detects v2 by [`MAGIC`]; otherwise treats `enc` as a v1 monolithic
/// envelope (decrypt → JSON → base64-decode `data`). Used by the server-side
/// consumers that need the whole media payload in memory (they already did so
/// for v1, so v2 does not make them worse).
pub fn decrypt_photo_blob(key: &[u8; 32], enc: &[u8]) -> Result<Vec<u8>, String> {
    if is_chunked(enc) {
        decrypt_chunked_to_bytes(key, enc)
    } else {
        decrypt_v1_to_bytes(key, enc)
    }
}

fn decrypt_v1_to_bytes(key: &[u8; 32], enc: &[u8]) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let plaintext = crypto::decrypt(key, enc)?;
    let envelope: serde_json::Value =
        serde_json::from_slice(&plaintext).map_err(|e| format!("v1 envelope JSON: {e}"))?;
    let data_b64 = envelope["data"]
        .as_str()
        .ok_or_else(|| "missing 'data' field in v1 blob envelope".to_string())?;
    base64::engine::general_purpose::STANDARD
        .decode(data_b64)
        .map_err(|e| format!("base64 decode: {e}"))
}

/// Stream-decrypt an encrypted blob **file** to a plaintext output **file** with
/// bounded memory. **Synchronous and blocking** — run inside `spawn_blocking`.
///
/// For v2 the chunk frames are read, decrypted, and written one at a time, so a
/// multi-gigabyte video never lives in memory. For v1 the whole blob is read and
/// decrypted (only used for files below the chunked threshold, so that's small).
///
/// On error the partial output is removed so a failed decrypt never leaves a
/// truncated plaintext file behind.
pub fn decrypt_blob_file_to_file(
    key: &[u8; 32],
    src: &Path,
    dst: &Path,
) -> Result<(), String> {
    match decrypt_blob_file_to_file_inner(key, src, dst) {
        Ok(()) => Ok(()),
        Err(e) => {
            if let Err(rm) = std::fs::remove_file(dst) {
                if rm.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path = ?dst, error = %rm, "[CHUNKED] failed to clean up partial plaintext");
                }
            }
            Err(e)
        }
    }
}

fn decrypt_blob_file_to_file_inner(
    key: &[u8; 32],
    src: &Path,
    dst: &Path,
) -> Result<(), String> {
    use std::io::{BufReader, BufWriter};

    let mut input = BufReader::new(std::fs::File::open(src).map_err(|e| format!("open enc src: {e}"))?);

    // Peek the magic to choose the path.
    let mut magic = [0u8; MAGIC.len()];
    let head = read_filled(&mut input, &mut magic).map_err(|e| format!("read magic: {e}"))?;

    if head == MAGIC.len() && magic == MAGIC {
        // v2 chunked — stream frame-by-frame.
        let out_file = std::fs::File::create(dst).map_err(|e| format!("create dst: {e}"))?;
        let mut out = BufWriter::new(out_file);

        // Skip the metadata frame (the media bytes don't need it).
        let mut len_buf = [0u8; 4];
        input
            .read_exact(&mut len_buf)
            .map_err(|e| format!("read meta len: {e}"))?;
        let meta_len = u32::from_be_bytes(len_buf) as usize;
        let mut meta = vec![0u8; meta_len];
        input
            .read_exact(&mut meta)
            .map_err(|e| format!("read meta: {e}"))?;
        drop(meta);

        loop {
            // A clean EOF at a frame boundary ends the stream.
            if !read_len_or_eof(&mut input, &mut len_buf).map_err(|e| format!("read frame len: {e}"))? {
                break;
            }
            let frame_len = u32::from_be_bytes(len_buf) as usize;
            let mut frame = vec![0u8; frame_len];
            input
                .read_exact(&mut frame)
                .map_err(|e| format!("read frame: {e}"))?;
            let plain = crypto::decrypt(key, &frame)?;
            out.write_all(&plain).map_err(|e| format!("write dst: {e}"))?;
        }
        out.flush().map_err(|e| format!("flush dst: {e}"))?;
        Ok(())
    } else {
        // v1 monolithic — small (below the chunked threshold), read whole.
        drop(input);
        let enc = std::fs::read(src).map_err(|e| format!("read v1 src: {e}"))?;
        let bytes = decrypt_v1_to_bytes(key, &enc)?;
        std::fs::write(dst, &bytes).map_err(|e| format!("write dst: {e}"))?;
        Ok(())
    }
}

fn decrypt_chunked_to_bytes(key: &[u8; 32], enc: &[u8]) -> Result<Vec<u8>, String> {
    let mut cur = MAGIC.len();
    // Skip the metadata frame — callers wanting media bytes don't need it.
    let meta_len = read_u32_be(enc, &mut cur)? as usize;
    let _ = take(enc, &mut cur, meta_len)?;

    let mut out: Vec<u8> = Vec::new();
    while cur < enc.len() {
        let frame_len = read_u32_be(enc, &mut cur)? as usize;
        let frame = take(enc, &mut cur, frame_len)?;
        let chunk = crypto::decrypt(key, frame)?;
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

// ── byte-stream helpers ──────────────────────────────────────────────────────

fn write_tracked(
    out: &mut impl Write,
    bytes: &[u8],
    hasher: &mut Sha256,
    written: &mut u64,
) -> Result<(), String> {
    out.write_all(bytes).map_err(|e| format!("write blob: {e}"))?;
    hasher.update(bytes);
    *written += bytes.len() as u64;
    Ok(())
}

/// Read a 4-byte length prefix, distinguishing a clean EOF (no more frames)
/// from a truncated one. Returns `Ok(true)` when `buf` was filled, `Ok(false)`
/// on a clean EOF at the start, and `Err` on a partial read.
fn read_len_or_eof(r: &mut impl Read, buf: &mut [u8; 4]) -> std::io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => {
                if filled == 0 {
                    return Ok(false); // clean end of frames
                }
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "truncated chunked blob (partial length prefix)",
                ));
            }
            Ok(n) => filled += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// Read until `buf` is full or EOF, tolerating short reads and `Interrupted`.
/// Returns the number of bytes actually read (`< buf.len()` only at EOF).
fn read_filled(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

fn read_u32_be(buf: &[u8], cur: &mut usize) -> Result<u32, String> {
    let end = cur
        .checked_add(4)
        .ok_or_else(|| "length prefix overflow".to_string())?;
    if end > buf.len() {
        return Err("truncated chunked blob (length prefix)".into());
    }
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&buf[*cur..end]);
    *cur = end;
    Ok(u32::from_be_bytes(arr))
}

fn take<'a>(buf: &'a [u8], cur: &mut usize, len: usize) -> Result<&'a [u8], String> {
    let end = cur
        .checked_add(len)
        .ok_or_else(|| "frame length overflow".to_string())?;
    if end > buf.len() {
        return Err("truncated chunked blob (frame body)".into());
    }
    let slice = &buf[*cur..end];
    *cur = end;
    Ok(slice)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("sp_chunked_test_{}_{}", std::process::id(), name))
    }

    #[test]
    fn roundtrip_multi_chunk() {
        let key = [0x11u8; 32]; // codeql[rust/hard-coded-cryptographic-value] -- test fixture
        // 2.5 chunks worth of pseudo-random data.
        let mut data = Vec::with_capacity(CHUNK_SIZE * 2 + 123);
        for i in 0..(CHUNK_SIZE * 2 + 123) {
            data.push((i as u8).wrapping_mul(31).wrapping_add(7));
        }
        let src = tmp_path("src.bin");
        let dst = tmp_path("dst.spb2");
        std::fs::write(&src, &data).unwrap();

        let meta = br#"{"v":2,"filename":"big.mp4","mime_type":"video/mp4"}"#;
        let res = encrypt_file_chunked(&key, &src, &dst, meta).unwrap();

        let blob = std::fs::read(&dst).unwrap();
        assert_eq!(res.blob_size as usize, blob.len());
        assert!(is_chunked(&blob));

        // Blob hash matches an independent digest of the written container.
        let expect_blob: [u8; 32] = Sha256::digest(&blob).into();
        assert_eq!(res.blob_sha256, expect_blob);

        // Round-trips back to the exact bytes.
        let out = decrypt_photo_blob(&key, &blob).unwrap();
        assert_eq!(out, data);

        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);
    }

    #[test]
    fn roundtrip_empty_and_small() {
        let key = [0x22u8; 32]; // codeql[rust/hard-coded-cryptographic-value] -- test fixture
        for size in [0usize, 1, 1024] {
            let data: Vec<u8> = (0..size).map(|i| i as u8).collect();
            let src = tmp_path(&format!("s{size}.bin"));
            let dst = tmp_path(&format!("d{size}.spb2"));
            std::fs::write(&src, &data).unwrap();
            encrypt_file_chunked(&key, &src, &dst, b"{}").unwrap();
            let blob = std::fs::read(&dst).unwrap();
            let out = decrypt_photo_blob(&key, &blob).unwrap();
            assert_eq!(out, data, "size {size}");
            let _ = std::fs::remove_file(&src);
            let _ = std::fs::remove_file(&dst);
        }
    }

    #[test]
    fn file_to_file_roundtrip_v2() {
        let key = [0x66u8; 32]; // codeql[rust/hard-coded-cryptographic-value] -- test fixture
        let mut data = Vec::with_capacity(CHUNK_SIZE + 999);
        for i in 0..(CHUNK_SIZE + 999) {
            data.push((i as u8).wrapping_mul(17).wrapping_add(3));
        }
        let src = tmp_path("f2f_src.bin");
        let blob = tmp_path("f2f_blob.spb2");
        let out = tmp_path("f2f_out.bin");
        std::fs::write(&src, &data).unwrap();
        encrypt_file_chunked(&key, &src, &blob, b"{\"v\":2}").unwrap();

        decrypt_blob_file_to_file(&key, &blob, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), data);

        for p in [&src, &blob, &out] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn file_to_file_roundtrip_v1() {
        use base64::Engine;
        let key = [0x77u8; 32]; // codeql[rust/hard-coded-cryptographic-value] -- test fixture
        let media = vec![0xABu8; 4096];
        // Build a v1 monolithic envelope blob: AES-GCM(JSON{...,"data":base64}).
        let envelope = serde_json::json!({
            "v": 1,
            "mime_type": "image/jpeg",
            "data": base64::engine::general_purpose::STANDARD.encode(&media),
        });
        let json = serde_json::to_vec(&envelope).unwrap();
        let enc = crypto::encrypt(&key, &json).unwrap();
        let blob = tmp_path("v1_blob.bin");
        let out = tmp_path("v1_out.bin");
        std::fs::write(&blob, &enc).unwrap();

        assert!(!is_chunked(&enc));
        decrypt_blob_file_to_file(&key, &blob, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), media);

        for p in [&blob, &out] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn wrong_key_fails_cleanly() {
        let key = [0x33u8; 32]; // codeql[rust/hard-coded-cryptographic-value] -- test fixture
        let bad = [0x44u8; 32]; // codeql[rust/hard-coded-cryptographic-value] -- test fixture
        let data = vec![9u8; 5000];
        let src = tmp_path("wk_src.bin");
        let dst = tmp_path("wk_dst.spb2");
        std::fs::write(&src, &data).unwrap();
        encrypt_file_chunked(&key, &src, &dst, b"{}").unwrap();
        let blob = std::fs::read(&dst).unwrap();
        assert!(decrypt_photo_blob(&bad, &blob).is_err());
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);
    }

    #[test]
    fn truncated_blob_errors_not_panics() {
        let key = [0x55u8; 32]; // codeql[rust/hard-coded-cryptographic-value] -- test fixture
        let data = vec![3u8; 9000];
        let src = tmp_path("tr_src.bin");
        let dst = tmp_path("tr_dst.spb2");
        std::fs::write(&src, &data).unwrap();
        encrypt_file_chunked(&key, &src, &dst, b"{}").unwrap();
        let mut blob = std::fs::read(&dst).unwrap();
        blob.truncate(blob.len() - 10); // chop the last frame
        assert!(decrypt_photo_blob(&key, &blob).is_err());
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);
    }
}
