use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::str::FromStr;

use crate::config::DatabaseConfig;

pub async fn init_pool(config: &DatabaseConfig) -> anyhow::Result<SqlitePool> {
    // Ensure parent directory exists
    if let Some(parent) = config.path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let options = SqliteConnectOptions::from_str(config.path.to_str().unwrap_or("simple-photos.db"))?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(config.max_connections)
        .connect_with(options)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    tracing::info!("Database initialized at {:?}", config.path);
    Ok(pool)
}
