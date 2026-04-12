//! Background worker that packages a user's media library into zip archives.
//!
//! Called from the handler via `tokio::spawn`. Reads all blobs and metadata
//! for the user, decrypts them, writes them into one or more zip files
//! (split at the configured size_limit), and records each file in
//! `export_files`.

use std::io::Write;
use std::path::PathBuf;

use sqlx::SqlitePool;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// Run the export job: read all blobs + metadata for the user, decrypt, produce zips.
pub async fn run_export(
    pool: SqlitePool,
    read_pool: SqlitePool,
    storage_root: PathBuf,
    user_id: String,
    job_id: String,
    size_limit: i64,
    jwt_secret: String,
) {
    // Mark job as running
    if let Err(e) = sqlx::query("UPDATE export_jobs SET status = 'running' WHERE id = ?")
        .bind(&job_id)
        .execute(&pool)
        .await
    {
        tracing::error!(job_id = %job_id, error = %e, "Failed to mark export job running");
        return;
    }

    match do_export(&pool, &read_pool, &storage_root, &user_id, &job_id, size_limit, &jwt_secret).await {
        Ok(()) => {
            let now = chrono::Utc::now().to_rfc3339();
            let _ = sqlx::query(
                "UPDATE export_jobs SET status = 'completed', completed_at = ? WHERE id = ?",
            )
            .bind(&now)
            .bind(&job_id)
            .execute(&pool)
            .await;
            tracing::info!(job_id = %job_id, "Export job completed");
        }
        Err(e) => {
            let error_msg = format!("{}", e);
            let now = chrono::Utc::now().to_rfc3339();
            let _ = sqlx::query(
                "UPDATE export_jobs SET status = 'failed', completed_at = ?, error = ? WHERE id = ?",
            )
            .bind(&now)
            .bind(&error_msg)
            .bind(&job_id)
            .execute(&pool)
            .await;
            tracing::error!(job_id = %job_id, error = %e, "Export job failed");
        }
    }
}

async fn do_export(
    pool: &SqlitePool,
    read_pool: &SqlitePool,
    storage_root: &PathBuf,
    user_id: &str,
    job_id: &str,
    size_limit: i64,
    jwt_secret: &str,
) -> Result<(), anyhow::Error> {
    // Ensure export directory exists
    let export_dir = storage_root.join("exports").join(job_id);
    tokio::fs::create_dir_all(&export_dir).await?;

    // Load the encryption key so we can decrypt blobs for the export.
    let encryption_key = crate::crypto::load_wrapped_key(read_pool, jwt_secret)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to load encryption key: {e}"))?
        .ok_or_else(|| anyhow::anyhow!(
            "No encryption key stored on this server. \
             Cannot produce a decrypted export."
        ))?;

    // Fetch all blobs for this user (id, blob_type, storage_path, size_bytes, client_hash, upload_time)
    let blobs: Vec<(String, String, String, i64, Option<String>, String)> = sqlx::query_as(
        "SELECT id, blob_type, storage_path, size_bytes, client_hash, upload_time \
         FROM blobs WHERE user_id = ? ORDER BY upload_time ASC",
    )
    .bind(user_id)
    .fetch_all(read_pool)
    .await?;

    // Build a map from blob_id → original filename (from the photos table).
    // The photo blob_id is stored in `encrypted_blob_id` and thumbnails in
    // `encrypted_thumb_blob_id`.
    let filename_rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT encrypted_blob_id, filename, mime_type FROM photos WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(read_pool)
    .await
    .unwrap_or_default();

    let filename_map: std::collections::HashMap<String, (String, String)> = filename_rows
        .into_iter()
        .map(|(blob_id, filename, mime)| (blob_id, (filename, mime)))
        .collect();

    if blobs.is_empty() {
        // Nothing to export — complete with 0 files
        return Ok(());
    }

    // Create a manifest with blob metadata (written into each zip)
    let manifest_entries: Vec<serde_json::Value> = blobs
        .iter()
        .map(|(id, blob_type, _path, size, hash, time)| {
            serde_json::json!({
                "blob_id": id,
                "blob_type": blob_type,
                "size_bytes": size,
                "client_hash": hash,
                "upload_time": time,
            })
        })
        .collect();

    let manifest_json = serde_json::to_string_pretty(&serde_json::json!({
        "export_version": 1,
        "user_id": user_id,
        "blob_count": blobs.len(),
        "exported_at": chrono::Utc::now().to_rfc3339(),
        "blobs": manifest_entries,
    }))?;

    // Collect blob data from disk (async I/O) and then hand off to
    // spawn_blocking for the CPU-bound + synchronous ZIP compression.
    // This prevents the zip writer from starving the tokio runtime —
    // previously, synchronous write_all / Deflate calls blocked async
    // worker threads, causing concurrent blob downloads (viewer
    // scrolling) to hang.

    // Phase 1: Read all blobs from disk using async I/O, then decrypt.
    struct BlobEntry {
        blob_id: String,
        blob_type: String,
        /// Decrypted data size (used for zip splitting).
        size_bytes: i64,
        /// Decrypted file data.
        data: Vec<u8>,
        /// Original filename if known (from photos table).
        original_filename: Option<String>,
    }

    let mut entries: Vec<BlobEntry> = Vec::with_capacity(blobs.len());
    let mut decrypt_failures = 0u32;

    for (blob_id, blob_type, storage_path, _size_bytes, _client_hash, _upload_time) in &blobs {
        let blob_path = storage_root.join(storage_path);

        // Skip files that don't exist on disk
        if !tokio::fs::try_exists(&blob_path).await.unwrap_or(false) {
            tracing::warn!(job_id = %job_id, blob_id = %blob_id, "Blob file missing, skipping");
            continue;
        }

        // Read encrypted blob file (async)
        let encrypted_data = match tokio::fs::read(&blob_path).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(job_id = %job_id, blob_id = %blob_id, error = %e, "Failed to read blob, skipping");
                continue;
            }
        };

        // Decrypt blob data
        let data = match crate::crypto::decrypt(&encryption_key, &encrypted_data) {
            Ok(d) => d,
            Err(e) => {
                decrypt_failures += 1;
                tracing::warn!(
                    job_id = %job_id, blob_id = %blob_id, error = %e,
                    "Failed to decrypt blob, skipping"
                );
                continue;
            }
        };

        // Look up original filename
        let original_filename = filename_map.get(blob_id).map(|(name, _)| name.clone());

        entries.push(BlobEntry {
            blob_id: blob_id.clone(),
            blob_type: blob_type.clone(),
            size_bytes: data.len() as i64,
            data,
            original_filename,
        });
    }

    if decrypt_failures > 0 {
        tracing::warn!(
            job_id = %job_id,
            decrypt_failures,
            "Some blobs could not be decrypted and were skipped"
        );
    }

    // Phase 2: Write all ZIP archives on a blocking thread so the async
    // runtime stays responsive for concurrent HTTP requests.
    let export_dir_clone = export_dir.clone();
    let manifest_clone = manifest_json.clone();
    let size_limit_clone = size_limit;

    let part_counts = tokio::task::spawn_blocking(move || -> Result<Vec<u32>, anyhow::Error> {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .compression_level(Some(1));

        let mut part_number = 1u32;
        let mut current_zip_size: i64 = 0;
        let mut current_zip = new_zip_writer(&export_dir_clone, part_number)?;
        let mut finished_parts: Vec<u32> = Vec::new();

        // Track used zip entry names to avoid duplicates (e.g. two photos
        // named "IMG_001.jpg"). Key = normalised entry name, value = count.
        let mut used_names: std::collections::HashMap<String, u32> =
            std::collections::HashMap::new();

        // Write manifest into the first zip
        let manifest_bytes = manifest_clone.as_bytes();
        current_zip.start_file("manifest.json", options)?;
        current_zip.write_all(manifest_bytes)?;
        current_zip_size += manifest_bytes.len() as i64;

        for entry in &entries {
            // Skip thumbnail blobs — export should only contain original media
            if entry.blob_type == "thumbnail" || entry.blob_type == "video_thumbnail" {
                continue;
            }

            // Check if adding this blob would exceed the size limit
            if current_zip_size + entry.size_bytes + 100 > size_limit_clone
                && current_zip_size > 0
            {
                let file = current_zip.finish()?;
                file.sync_all()?;
                drop(file);
                finished_parts.push(part_number);

                part_number += 1;
                current_zip = new_zip_writer(&export_dir_clone, part_number)?;
                current_zip_size = 0;
                // Reset used names for the new zip part — each zip is
                // independent so names only need to be unique within a part.
                used_names.clear();
            }

            // Use original filename when available, otherwise fall back to
            // blob_id with an appropriate extension.
            let filename = if let Some(ref orig) = entry.original_filename {
                orig.clone()
            } else {
                let ext = match entry.blob_type.as_str() {
                    "photo" => "jpg",
                    "gif" => "gif",
                    "video" => "mp4",
                    "audio" => "mp3",
                    "album_manifest" => "json",
                    _ => "dat",
                };
                format!("{}.{}", entry.blob_id, ext)
            };

            // Organise into sub-folders by type.
            let zip_entry_name = match entry.blob_type.as_str() {
                "album_manifest" => format!("metadata/{}", filename),
                _ => format!("photos/{}", filename),
            };

            // Deduplicate: if this name was already used in the current zip,
            // append a counter before the extension (e.g. "photo_(2).jpg").
            let unique_name = {
                let count = used_names.entry(zip_entry_name.clone()).or_insert(0);
                *count += 1;
                if *count == 1 {
                    zip_entry_name
                } else {
                    // Split at the last '.' to insert the counter before the extension
                    if let Some(dot_pos) = zip_entry_name.rfind('.') {
                        format!(
                            "{}_({}){}", &zip_entry_name[..dot_pos], count, &zip_entry_name[dot_pos..]
                        )
                    } else {
                        format!("{}_({})", zip_entry_name, count)
                    }
                }
            };

            current_zip.start_file(&unique_name, options)?;
            current_zip.write_all(&entry.data)?;
            current_zip_size += entry.data.len() as i64;
        }

        // Finalize the last zip
        let file = current_zip.finish()?;
        file.sync_all()?;
        drop(file);
        finished_parts.push(part_number);

        Ok(finished_parts)
    })
    .await
    .map_err(|e| anyhow::anyhow!("ZIP compression task panicked: {e}"))??;

    // Phase 3: Register all zip parts in the database (async).
    for part in part_counts {
        register_zip_file(pool, job_id, &export_dir, part).await?;
    }

    Ok(())
}

fn new_zip_writer(
    export_dir: &PathBuf,
    part_number: u32,
) -> Result<zip::ZipWriter<std::fs::File>, anyhow::Error> {
    let filename = format!("export_part_{:03}.zip", part_number);
    let path = export_dir.join(&filename);
    let file = std::fs::File::create(&path)?;
    Ok(zip::ZipWriter::new(file))
}

async fn register_zip_file(
    pool: &SqlitePool,
    job_id: &str,
    export_dir: &PathBuf,
    part_number: u32,
) -> Result<(), anyhow::Error> {
    let filename = format!("export_part_{:03}.zip", part_number);
    let full_path = export_dir.join(&filename);

    let size_bytes = tokio::fs::metadata(&full_path)
        .await
        .map(|m| m.len() as i64)
        .unwrap_or(0);

    let file_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    let created_at = now.to_rfc3339();
    let expires_at = (now + chrono::Duration::hours(24)).to_rfc3339();

    // Store relative path from storage root
    let job_id_for_path = job_id.to_string();
    let relative_path = format!("exports/{}/{}", job_id_for_path, filename);

    sqlx::query(
        "INSERT INTO export_files (id, job_id, filename, file_path, size_bytes, created_at, expires_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&file_id)
    .bind(job_id)
    .bind(&filename)
    .bind(&relative_path)
    .bind(size_bytes)
    .bind(&created_at)
    .bind(&expires_at)
    .execute(pool)
    .await?;

    Ok(())
}
