//! Background worker that packages a user's media library into zip archives.
//!
//! Called from the handler via `tokio::spawn`. Reads all blobs and metadata
//! for the user, writes them into one or more zip files (split at the
//! configured size_limit), and records each file in `export_files`.

use std::io::Write;
use std::path::PathBuf;

use sqlx::SqlitePool;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// Run the export job: read all blobs + metadata for the user, produce zips.
pub async fn run_export(
    pool: SqlitePool,
    read_pool: SqlitePool,
    storage_root: PathBuf,
    user_id: String,
    job_id: String,
    size_limit: i64,
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

    match do_export(&pool, &read_pool, &storage_root, &user_id, &job_id, size_limit).await {
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
) -> Result<(), anyhow::Error> {
    // Ensure export directory exists
    let export_dir = storage_root.join("exports").join(job_id);
    tokio::fs::create_dir_all(&export_dir).await?;

    // Fetch all blobs for this user (id, blob_type, storage_path, size_bytes, client_hash, upload_time)
    let blobs: Vec<(String, String, String, i64, Option<String>, String)> = sqlx::query_as(
        "SELECT id, blob_type, storage_path, size_bytes, client_hash, upload_time \
         FROM blobs WHERE user_id = ? ORDER BY upload_time ASC",
    )
    .bind(user_id)
    .fetch_all(read_pool)
    .await?;

    // Fetch metadata file paths for this user
    let metadata_files: Vec<(String, String)> = sqlx::query_as(
        "SELECT pm.blob_id, pm.metadata_path FROM photo_metadata pm \
         WHERE pm.user_id = ? AND pm.metadata_path IS NOT NULL",
    )
    .bind(user_id)
    .fetch_all(read_pool)
    .await
    .unwrap_or_default();

    let metadata_map: std::collections::HashMap<String, String> =
        metadata_files.into_iter().collect();

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

    // Phase 1: Read all blobs from disk using async I/O.
    struct BlobEntry {
        blob_id: String,
        blob_type: String,
        size_bytes: i64,
        data: Vec<u8>,
        meta_data: Option<Vec<u8>>,
    }

    let mut entries: Vec<BlobEntry> = Vec::with_capacity(blobs.len());
    for (blob_id, blob_type, storage_path, size_bytes, _client_hash, _upload_time) in &blobs {
        let blob_path = storage_root.join(storage_path);

        // Skip files that don't exist on disk
        if !tokio::fs::try_exists(&blob_path).await.unwrap_or(false) {
            tracing::warn!(job_id = %job_id, blob_id = %blob_id, "Blob file missing, skipping");
            continue;
        }

        // Read blob file (async)
        let data = match tokio::fs::read(&blob_path).await {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(job_id = %job_id, blob_id = %blob_id, error = %e, "Failed to read blob, skipping");
                continue;
            }
        };

        // Read metadata if available (async)
        let meta_data = if let Some(meta_rel_path) = metadata_map.get(blob_id) {
            let meta_path = storage_root.join(meta_rel_path);
            tokio::fs::read(&meta_path).await.ok()
        } else {
            None
        };

        entries.push(BlobEntry {
            blob_id: blob_id.clone(),
            blob_type: blob_type.clone(),
            size_bytes: *size_bytes,
            data,
            meta_data,
        });
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

        // Write manifest into the first zip
        let manifest_bytes = manifest_clone.as_bytes();
        current_zip.start_file("manifest.json", options)?;
        current_zip.write_all(manifest_bytes)?;
        current_zip_size += manifest_bytes.len() as i64;

        for entry in &entries {
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
            }

            let zip_entry_name = format!("blobs/{}/{}.bin", entry.blob_type, entry.blob_id);
            current_zip.start_file(&zip_entry_name, options)?;
            current_zip.write_all(&entry.data)?;
            current_zip_size += entry.data.len() as i64;

            if let Some(ref meta_data) = entry.meta_data {
                let meta_zip_name = format!("metadata/{}.json", entry.blob_id);
                current_zip.start_file(&meta_zip_name, options)?;
                current_zip.write_all(meta_data)?;
                current_zip_size += meta_data.len() as i64;
            }
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
