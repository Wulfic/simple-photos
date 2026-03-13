//! SQLite connection pool initialization and migration runner.
//!
//! Creates the database file if missing, configures WAL journal mode
//! for concurrent read/write performance, and runs all embedded SQL
//! migrations from the `./migrations` directory.

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

use crate::config::DatabaseConfig;

/// Create a SQLite connection pool, configure it, run all pending
/// migrations, and return the pool ready for use by handlers.
pub async fn init_pool(config: &DatabaseConfig) -> anyhow::Result<SqlitePool> {
    // Ensure the parent directory exists (e.g. "data/" for "data/db/simple-photos.db")
    if let Some(parent) = config.path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let options = SqliteConnectOptions::from_str(config.path.to_str().unwrap_or("simple-photos.db"))?
        .create_if_missing(true)
        // WAL mode enables concurrent reads during writes — critical for a
        // multi-handler web server where reads heavily outnumber writes.
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        // Enforce referential integrity so CASCADE deletes work correctly.
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(config.max_connections)
        .connect_with(options)
        .await?;

    // Run all SQL migrations from ./migrations (embedded at compile time by sqlx).
    // New migrations are applied automatically on each server start.
    sqlx::migrate!("./migrations").run(&pool).await?;

    tracing::info!("Database initialized at {:?}", config.path);
    Ok(pool)
}
