//! External diagnostics API — authenticated via HTTP Basic Auth.
//!
//! These endpoints let other servers (monitoring systems, dashboards, etc.)
//! pull diagnostics data using a valid admin username and password.
//!
//! Authentication: standard HTTP Basic Auth header
//!   `Authorization: Basic base64(username:password)`
//!
//! All endpoints require admin role.
//!
//! **Security note:** These endpoints perform bcrypt verification on every
//! request but have **no rate limiting or account lockout**. In production,
//! protect them behind a reverse proxy or firewall rule.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use std::collections::HashMap;
use std::time::Instant;

use crate::error::AppError;
use crate::state::AppState;

use super::handlers::{disk_stats, read_cpu_seconds, read_rss_bytes, server_start};
use super::models::*;

// ── Basic Auth Helper ─────────────────────────────────────────────────────

/// Validate HTTP Basic Auth credentials against the users table.
/// Returns the user ID if valid and the user has admin role.
async fn require_basic_auth_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<String, AppError> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AppError::Unauthorized("Missing Authorization header. Use Basic Auth.".into())
        })?;

    // Parse "Basic base64(username:password)"
    let encoded = auth_header
        .strip_prefix("Basic ")
        .or_else(|| auth_header.strip_prefix("basic "))
        .ok_or_else(|| {
            AppError::Unauthorized("Invalid Authorization header. Expected Basic Auth.".into())
        })?;

    let decoded = base64_decode(encoded)
        .map_err(|_| AppError::Unauthorized("Invalid Base64 in Authorization header".into()))?;

    let (username, password) = decoded
        .split_once(':')
        .ok_or_else(|| AppError::Unauthorized("Invalid Basic Auth format".into()))?;

    if username.is_empty() || password.is_empty() {
        return Err(AppError::Unauthorized(
            "Username and password are required".into(),
        ));
    }

    // Look up user
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT id, password_hash, COALESCE(role, 'user') FROM users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(&state.read_pool)
    .await?;

    let (user_id, password_hash, role) = match row {
        Some(r) => r,
        None => {
            // Constant-time: still run bcrypt to prevent timing attacks
            let _ = bcrypt::verify(
                password,
                "$2b$12$LJ3m9blCPMEtJDZk4CYOqe4CIH55aN38bwSqggfgA1mJm/kzbyPhK",
            );
            return Err(AppError::Unauthorized(
                "Invalid username or password".into(),
            ));
        }
    };

    let valid = bcrypt::verify(password, &password_hash)
        .map_err(|e| AppError::Internal(format!("bcrypt error: {}", e)))?;

    if !valid {
        return Err(AppError::Unauthorized(
            "Invalid username or password".into(),
        ));
    }

    if role != "admin" {
        return Err(AppError::Forbidden("Admin access required".into()));
    }

    Ok(user_id)
}

/// Simple base64 decoder (standard alphabet, no padding required).
fn base64_decode(input: &str) -> Result<String, ()> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(input.trim())
        .map_err(|_| ())?;
    String::from_utf8(bytes).map_err(|_| ())
}

// ═══════════════════════════════════════════════════════════════════════════
// GET /api/external/diagnostics/health — lightweight health for monitoring
// ═══════════════════════════════════════════════════════════════════════════

pub async fn external_health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ExternalHealthResponse>, AppError> {
    require_basic_auth_admin(&state, &headers).await?;

    let (start_instant, started_at) = server_start();
    let uptime = start_instant.elapsed().as_secs();

    let pool = &state.read_pool;

    // DB ping
    let t0 = Instant::now();
    let _: i64 = sqlx::query_scalar("SELECT 1")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let db_ping_ms = t0.elapsed().as_secs_f64() * 1000.0;

    // Disk usage
    let storage_root = (**state.storage_root.load()).clone();
    let (disk_total, disk_available) = disk_stats(&storage_root);
    let disk_used_percent = if disk_total > 0 {
        ((disk_total - disk_available) as f64 / disk_total as f64) * 100.0
    } else {
        0.0
    };

    // Counts
    let total_photos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let total_users: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    Ok(Json(ExternalHealthResponse {
        status: "ok".into(),
        version: crate::VERSION.into(),
        uptime_seconds: uptime,
        started_at: started_at.clone(),
        memory_rss_bytes: read_rss_bytes(),
        cpu_seconds: read_cpu_seconds(),
        db_ping_ms,
        disk_used_percent,
        total_photos,
        total_users,
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// GET /api/external/diagnostics — full metrics (same as admin endpoint)
// ═══════════════════════════════════════════════════════════════════════════

pub async fn external_full(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::response::Response, AppError> {
    use axum::response::IntoResponse;

    require_basic_auth_admin(&state, &headers).await?;

    let storage_root = (**state.storage_root.load()).clone();
    let resp = super::collect::collect_full_diagnostics(
        &state.read_pool,
        &state.config,
        &storage_root,
    )
    .await;

    Ok(Json(resp).into_response())
}

// ═══════════════════════════════════════════════════════════════════════════
// GET /api/external/diagnostics/storage — storage-focused metrics
// ═══════════════════════════════════════════════════════════════════════════

pub async fn external_storage(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ExternalStorageResponse>, AppError> {
    require_basic_auth_admin(&state, &headers).await?;

    let pool = &state.read_pool;
    let storage_root = (**state.storage_root.load()).clone();

    let storage = super::collect::collect_storage_stats(&storage_root).await;

    // Photo storage subset
    let total_photos: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM photos")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    let total_file_bytes: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM photos")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let total_thumb_bytes: i64 = 0;
    let media_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT media_type, COUNT(*) as cnt FROM photos GROUP BY media_type")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    let photos_by_media_type: HashMap<String, i64> = media_rows.into_iter().collect();

    let photos = ExternalPhotoStorageStats {
        total_photos,
        total_file_bytes,
        total_thumb_bytes,
        photos_by_media_type,
    };

    // Database size
    let db_path = &state.config.database.path;
    let db_size = tokio::fs::metadata(db_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let wal_path = db_path.with_extension("db-wal");
    let wal_size = tokio::fs::metadata(&wal_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);

    let database = ExternalDatabaseSize {
        size_bytes: db_size,
        wal_size_bytes: wal_size,
    };

    Ok(Json(ExternalStorageResponse {
        storage,
        photos,
        database,
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// GET /api/external/diagnostics/audit — audit & security summary
// ═══════════════════════════════════════════════════════════════════════════

pub async fn external_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ExternalAuditResponse>, AppError> {
    require_basic_auth_admin(&state, &headers).await?;

    let pool = &state.read_pool;

    let audit = super::collect::collect_audit_summary(pool).await;
    let users = super::collect::collect_user_stats(pool).await;
    let client_logs = super::collect::collect_client_log_summary(pool).await;

    Ok(Json(ExternalAuditResponse {
        audit,
        users,
        client_logs,
    }))
}
