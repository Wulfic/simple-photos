//! SQLite connection pool initialization and migration runner.
//!
//! Creates the database file if missing, configures WAL journal mode
//! for concurrent read/write performance, and runs all embedded SQL
//! migrations from the `./migrations` directory.
//!
//! ## Dual-Pool Architecture
//!
//! Two separate pools are created to prevent read/write contention:
//!
//! - **Write pool** (`pool`): Used for INSERT/UPDATE/DELETE and transactions.
//!   Limited connections (default 4) because SQLite only allows one writer
//!   at a time — more connections just increase lock contention.
//!
//! - **Read pool** (`read_pool`): Read-only connections for SELECT queries.
//!   Higher connection count (default 32) for maximum read parallelism.
//!   Uses `PRAGMA query_only = 1` to guarantee no accidental writes.
//!
//! This ensures gallery browsing (reads) is never starved by concurrent
//! uploads/backups (writes), even under heavy load.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

use crate::config::DatabaseConfig;

/// Shared SQLite PRAGMA tuning applied to both read and write pools.
fn base_options(config: &DatabaseConfig) -> anyhow::Result<SqliteConnectOptions> {
    Ok(
        SqliteConnectOptions::from_str(config.path.to_str().unwrap_or("simple-photos.db"))?
            .create_if_missing(true)
            // WAL mode enables concurrent reads during writes — critical for a
            // multi-handler web server where reads heavily outnumber writes.
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            // Enforce referential integrity so CASCADE deletes work correctly.
            .foreign_keys(true)
            // Wait up to 10 s for a write-lock instead of failing immediately.
            // Increased from 5 s to handle bursts of concurrent writes during
            // heavy upload/backup periods without "database is locked" errors.
            .busy_timeout(std::time::Duration::from_secs(10))
            // 16 MB page cache (negative = KB).  Keeps hot pages in memory so
            // repeated photo-list / thumbnail queries don't hit disk.
            .pragma("cache_size", "-16000")
            // NORMAL sync: one fewer fsync per transaction — safe with WAL
            // and gives a significant write throughput boost.
            .pragma("synchronous", "NORMAL")
            // Keep temp tables / indices in memory rather than a temp file.
            .pragma("temp_store", "MEMORY")
            // Enable memory-mapped I/O (256 MB) for faster reads of large DBs.
            .pragma("mmap_size", "268435456")
            // Increase WAL auto-checkpoint threshold from the default 1000 pages
            // to 2000 pages (~8 MB). This reduces checkpoint frequency during
            // burst writes (upload/backup), preventing checkpoint-induced reader
            // stalls. The WAL file grows slightly larger but checkpoints less often.
            .pragma("wal_autocheckpoint", "2000"),
    )
}

/// Create both the write and read connection pools, run migrations, and
/// return `(write_pool, read_pool)`.
///
/// The write pool is used for all INSERT/UPDATE/DELETE operations.
/// The read pool is used for SELECT queries in request handlers.
pub async fn init_pools(config: &DatabaseConfig) -> anyhow::Result<(SqlitePool, SqlitePool)> {
    // Ensure the parent directory exists
    if let Some(parent) = config.path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // ── Write pool ──────────────────────────────────────────────────────
    // Limited connections: SQLite allows only 1 concurrent writer, so excess
    // write connections just queue behind the write lock. 4 connections gives
    // enough headroom for pipelined transactions without excessive contention.
    let write_options = base_options(config)?;

    let write_pool_size = config.max_connections.min(8).max(2); // 2..=8
    let write_pool = SqlitePoolOptions::new()
        .max_connections(write_pool_size)
        .min_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect_with(write_options)
        .await?;

    // Run all SQL migrations (requires write access).
    // `set_ignore_missing` allows the server to start when the DB was previously
    // set up with more migration files than currently exist (e.g. after consolidation).
    sqlx::migrate!("./migrations")
        .set_ignore_missing(true)
        .run(&write_pool)
        .await?;

    // ── Read pool ───────────────────────────────────────────────────────
    // Many connections for maximum read parallelism. SQLite WAL allows
    // unlimited concurrent readers. `query_only = 1` prevents accidental
    // writes from leaking into the read pool.
    let read_options = base_options(config)?
        .pragma("query_only", "1");

    let read_pool_size = config.read_pool_max_connections;
    let read_pool = SqlitePoolOptions::new()
        .max_connections(read_pool_size)
        .min_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_with(read_options)
        .await?;

    tracing::info!(
        "Database initialized at {:?} (write pool: {}, read pool: {})",
        config.path,
        write_pool_size,
        read_pool_size,
    );

    Ok((write_pool, read_pool))
}
