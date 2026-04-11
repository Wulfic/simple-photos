//! Export endpoint handlers — trigger export, list jobs, list files, download.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

// ── Request / Response DTOs ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct StartExportRequest {
    /// Maximum size of each zip file in bytes.
    pub size_limit: i64,
}

#[derive(Serialize)]
pub struct ExportJobResponse {
    pub id: String,
    pub status: String,
    pub size_limit: i64,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct ExportFileResponse {
    pub id: String,
    pub job_id: String,
    pub filename: String,
    pub size_bytes: i64,
    pub created_at: String,
    pub expires_at: String,
    pub download_url: String,
}

#[derive(Serialize)]
pub struct ExportStatusResponse {
    pub job: ExportJobResponse,
    pub files: Vec<ExportFileResponse>,
}

#[derive(Serialize)]
pub struct ExportListResponse {
    pub jobs: Vec<ExportJobResponse>,
}

#[derive(Serialize)]
pub struct ExportFilesListResponse {
    pub files: Vec<ExportFileResponse>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `POST /api/export` — Start a new library export job.
pub async fn start_export(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(body): Json<StartExportRequest>,
) -> Result<Json<ExportJobResponse>, AppError> {
    // Validate size limit (min 1 GB, max 50 GB)
    let min_size: i64 = 1_073_741_824; // 1 GB
    let max_size: i64 = 53_687_091_200; // 50 GB
    if body.size_limit < min_size || body.size_limit > max_size {
        return Err(AppError::BadRequest(format!(
            "size_limit must be between {} and {} bytes",
            min_size, max_size
        )));
    }

    // Check if user already has an active export (pending/running)
    let active: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM export_jobs WHERE user_id = ? AND status IN ('pending', 'running') LIMIT 1",
    )
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?;

    if active.is_some() {
        return Err(AppError::Conflict(
            "An export is already in progress. Wait for it to complete or cancel it.".into(),
        ));
    }

    let job_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO export_jobs (id, user_id, status, size_limit, created_at) VALUES (?, ?, 'pending', ?, ?)",
    )
    .bind(&job_id)
    .bind(&auth.user_id)
    .bind(body.size_limit)
    .bind(&now)
    .execute(&state.pool)
    .await?;

    crate::audit::log(
        &state,
        crate::audit::AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({"action": "export_start", "job_id": &job_id, "size_limit": body.size_limit})),
    )
    .await;

    // Spawn the background worker
    let pool = state.pool.clone();
    let read_pool = state.read_pool.clone();
    let storage_root = state.storage_root.load().as_ref().clone();
    let user_id = auth.user_id.clone();
    let jid = job_id.clone();
    let size_limit = body.size_limit;

    tokio::spawn(async move {
        super::worker::run_export(pool, read_pool, storage_root, user_id, jid, size_limit).await;
    });

    Ok(Json(ExportJobResponse {
        id: job_id,
        status: "pending".into(),
        size_limit: body.size_limit,
        created_at: now,
        completed_at: None,
        error: None,
    }))
}

/// `GET /api/export/status` — Get the latest export job status for this user.
pub async fn export_status(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<ExportStatusResponse>, AppError> {
    let row: Option<(String, String, i64, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, status, size_limit, created_at, completed_at, error \
         FROM export_jobs WHERE user_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?;

    let (job_id, status, size_limit, created_at, completed_at, error) = row
        .ok_or_else(|| AppError::NotFound)?;

    // Only return files once the job is fully completed to prevent
    // downloading partially-written zip archives.
    let file_rows: Vec<(String, String, String, i64, String, String)> = if status == "completed" {
        sqlx::query_as(
            "SELECT id, job_id, filename, size_bytes, created_at, expires_at \
             FROM export_files WHERE job_id = ? ORDER BY filename",
        )
        .bind(&job_id)
        .fetch_all(&state.read_pool)
        .await?
    } else {
        Vec::new()
    };

    let now = chrono::Utc::now();
    let files: Vec<ExportFileResponse> = file_rows
        .into_iter()
        .filter(|(_, _, _, _, _, expires_at)| {
            chrono::DateTime::parse_from_rfc3339(expires_at)
                .map(|e| e > now)
                .unwrap_or(false)
        })
        .map(|(id, job_id, filename, size_bytes, created_at, expires_at)| ExportFileResponse {
            download_url: format!("/api/export/files/{}/download", id),
            id,
            job_id,
            filename,
            size_bytes,
            created_at,
            expires_at,
        })
        .collect();

    Ok(Json(ExportStatusResponse {
        job: ExportJobResponse {
            id: job_id,
            status,
            size_limit,
            created_at,
            completed_at,
            error,
        },
        files,
    }))
}

/// `GET /api/export/files` — List all non-expired export files for this user.
pub async fn list_export_files(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<ExportFilesListResponse>, AppError> {
    let now = chrono::Utc::now().to_rfc3339();
    let rows: Vec<(String, String, String, i64, String, String)> = sqlx::query_as(
        "SELECT ef.id, ef.job_id, ef.filename, ef.size_bytes, ef.created_at, ef.expires_at \
         FROM export_files ef \
         JOIN export_jobs ej ON ef.job_id = ej.id \
         WHERE ej.user_id = ? AND ej.status = 'completed' AND ef.expires_at > ? \
         ORDER BY ef.filename",
    )
    .bind(&auth.user_id)
    .bind(&now)
    .fetch_all(&state.read_pool)
    .await?;

    let files: Vec<ExportFileResponse> = rows
        .into_iter()
        .map(|(id, job_id, filename, size_bytes, created_at, expires_at)| ExportFileResponse {
            download_url: format!("/api/export/files/{}/download", id),
            id,
            job_id,
            filename,
            size_bytes,
            created_at,
            expires_at,
        })
        .collect();

    Ok(Json(ExportFilesListResponse { files }))
}

/// `GET /api/export/files/:id/download` — Download an export zip file.
pub async fn download_export_file(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(file_id): Path<String>,
) -> Result<Response, AppError> {
    let now = chrono::Utc::now().to_rfc3339();

    // Verify the file belongs to this user and hasn't expired
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT ef.filename, ef.file_path \
         FROM export_files ef \
         JOIN export_jobs ej ON ef.job_id = ej.id \
         WHERE ef.id = ? AND ej.user_id = ? AND ej.status = 'completed' AND ef.expires_at > ?",
    )
    .bind(&file_id)
    .bind(&auth.user_id)
    .bind(&now)
    .fetch_optional(&state.read_pool)
    .await?;

    let (filename, file_path) = row.ok_or_else(|| AppError::NotFound)?;

    let storage_root = state.storage_root.load();
    let full_path = storage_root.join(&file_path);

    if !tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
        return Err(AppError::NotFound);
    }

    let meta = tokio::fs::metadata(&full_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to stat export file: {}", e)))?;

    let file = tokio::fs::File::open(&full_path)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to open export file: {}", e)))?;

    let stream = tokio_util::io::ReaderStream::with_capacity(file, 256 * 1024);
    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/zip")
        .header(
            "Content-Disposition",
            HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename))
                .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=\"export.zip\"")),
        )
        .header("Content-Length", meta.len())
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?)
}

/// `DELETE /api/export/:job_id` — Cancel/delete an export job and its files.
pub async fn delete_export(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Result<StatusCode, AppError> {
    // Verify ownership
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM export_jobs WHERE id = ? AND user_id = ?",
    )
    .bind(&job_id)
    .bind(&auth.user_id)
    .fetch_optional(&state.read_pool)
    .await?;

    if row.is_none() {
        return Err(AppError::NotFound);
    }

    // Delete files from disk
    let file_paths: Vec<(String,)> = sqlx::query_as(
        "SELECT file_path FROM export_files WHERE job_id = ?",
    )
    .bind(&job_id)
    .fetch_all(&state.read_pool)
    .await?;

    let storage_root = state.storage_root.load();
    for (path,) in &file_paths {
        let full_path = storage_root.join(path);
        if tokio::fs::try_exists(&full_path).await.unwrap_or(false) {
            let _ = tokio::fs::remove_file(&full_path).await;
        }
    }

    // Delete DB records (CASCADE deletes export_files)
    sqlx::query("DELETE FROM export_jobs WHERE id = ?")
        .bind(&job_id)
        .execute(&state.pool)
        .await?;

    crate::audit::log(
        &state,
        crate::audit::AuditEvent::AdminAction,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({"action": "export_delete", "job_id": &job_id})),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
