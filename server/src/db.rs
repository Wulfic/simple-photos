//! SQLite connection pool initialization and migration runner.
//!
//! Creates the database file if missing, configures WAL journal mode
//! for concurrent read/write performance, and runs all embedded SQL
//! migrations from the `./migrations` directory.
//!
//! ## Encryption at rest (SQLCipher)
//!
//! The entire database file is encrypted at rest with SQLCipher (AES-256).
//! `libsqlite3-sys` is built with `bundled-sqlcipher-vendored-openssl`, and
//! every connection is unlocked with `PRAGMA key` before any other access.
//! The key is derived deterministically from the server's JWT secret — the
//! same secret the blob-encryption layer already depends on — so no extra
//! setup or key file is required, and a stolen `.db` file is useless without
//! the server config.
//!
//! Databases created before encryption was introduced are detected as
//! plaintext at boot and transparently migrated via `sqlcipher_export`, with
//! the original kept as a `*.pre-sqlcipher.bak` safety copy.
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

use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{ConnectOptions, Connection, SqlitePool};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use crate::config::DatabaseConfig;

/// Derive the SQLCipher passphrase from the JWT secret.  Returned as a 64-char
/// hex string (no quotes/special chars) so it round-trips cleanly through both
/// `PRAGMA key` and the `ATTACH ... KEY` statement used during migration.
pub fn derive_db_key(jwt_secret: &str) -> String {
    let hash = Sha256::digest(format!("simple-photos-db-encryption:{jwt_secret}").as_bytes());
    hex::encode(hash)
}

/// Render the key as a quoted SQL string literal for `PRAGMA key`.
///
/// sqlx interpolates pragma values **verbatim** (it does not quote them), so a
/// bare hex key beginning with a digit is mis-parsed by SQLite as a malformed
/// number ("unrecognized token") and the connection never opens.  Wrapping it
/// in single quotes forces unambiguous passphrase mode for every key.  The
/// `ATTACH ... KEY '…'` statement in the migration path quotes identically, so
/// the derived passphrase matches across both code paths.
fn key_pragma(db_key: &str) -> String {
    format!("'{}'", db_key.replace('\'', "''"))
}

/// Shared SQLite PRAGMA tuning applied to both read and write pools.
///
/// `PRAGMA key` is registered first; sqlx 0.8 guarantees the key pragma runs
/// before any built-in pragma (journal_mode, foreign_keys, …), which is what
/// SQLCipher requires.
fn base_options(config: &DatabaseConfig, db_key: &str) -> anyhow::Result<SqliteConnectOptions> {
    Ok(
        SqliteConnectOptions::from_str(config.path.to_str().unwrap_or("simple-photos.db"))?
            // SQLCipher unlock — must be applied before touching the schema.
            .pragma("key", key_pragma(db_key))
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
/// `jwt_secret` is used to derive the SQLCipher key; it is never stored.
pub async fn init_pools(
    config: &DatabaseConfig,
    jwt_secret: &str,
) -> anyhow::Result<(SqlitePool, SqlitePool)> {
    // Ensure the parent directory exists
    if let Some(parent) = config.path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let db_key = derive_db_key(jwt_secret);

    // Upgrade a legacy plaintext database in place before opening it encrypted.
    ensure_encrypted(&config.path, &db_key).await?;

    // ── Write pool ──────────────────────────────────────────────────────
    // Limited connections: SQLite allows only 1 concurrent writer, so excess
    // write connections just queue behind the write lock. 4 connections gives
    // enough headroom for pipelined transactions without excessive contention.
    let write_options = base_options(config, &db_key)?;

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
    let read_options = base_options(config, &db_key)?.pragma("query_only", "1");

    let read_pool_size = config.read_pool_max_connections;
    let read_pool = SqlitePoolOptions::new()
        .max_connections(read_pool_size)
        .min_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_with(read_options)
        .await?;

    tracing::info!(
        "Database initialized at {:?} (encrypted, write pool: {}, read pool: {})",
        config.path,
        write_pool_size,
        read_pool_size,
    );

    Ok((write_pool, read_pool))
}

/// Ensure the database at `path` is SQLCipher-encrypted with `db_key`.
///
/// * Missing file → nothing to do (it will be created encrypted).
/// * Opens with the key → already encrypted, nothing to do.
/// * Opens *without* a key → legacy plaintext DB, migrate it.
/// * Opens with neither → wrong `jwt_secret` or corrupt file → hard error
///   (refusing to start is far safer than silently creating a second DB).
async fn ensure_encrypted(path: &Path, db_key: &str) -> anyhow::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    if probe_open(path, Some(db_key)).await.is_ok() {
        return Ok(());
    }
    if probe_open(path, None).await.is_ok() {
        tracing::warn!(
            db = ?path,
            "Detected an unencrypted database — migrating it to SQLCipher (encrypted at rest)"
        );
        migrate_plaintext_to_encrypted(path, db_key).await?;
        return Ok(());
    }
    anyhow::bail!(
        "Database at {path:?} cannot be opened with or without the configured key. \
         This usually means auth.jwt_secret changed since the database was encrypted \
         (the key is derived from it), or the file is corrupt. Restore the original \
         jwt_secret or the *.pre-sqlcipher.bak backup."
    );
}

/// Try to open `path` (optionally keyed) and read its schema.  Returns `Ok`
/// only when the database is genuinely readable with the given key state.
async fn probe_open(path: &Path, key: Option<&str>) -> anyhow::Result<()> {
    let mut opts =
        SqliteConnectOptions::from_str(path.to_str().unwrap_or_default())?.create_if_missing(false);
    if let Some(k) = key {
        opts = opts.pragma("key", key_pragma(k));
    }
    let mut conn = opts.connect().await?;
    // SQLCipher only errors here ("file is not a database") when the key state
    // is wrong, so this SELECT is the actual decryption check.
    let result = sqlx::query("SELECT count(*) FROM sqlite_master")
        .execute(&mut conn)
        .await;
    let _ = conn.close().await;
    result.map(|_| ()).map_err(Into::into)
}

/// Migrate a plaintext SQLite database to an encrypted SQLCipher database
/// using `sqlcipher_export`, preserving the original as a `.pre-sqlcipher.bak`.
async fn migrate_plaintext_to_encrypted(path: &Path, db_key: &str) -> anyhow::Result<()> {
    let enc_path = sibling(path, ".sqlcipher-new");
    // Clear any leftover temp from a previous interrupted attempt.
    let _ = tokio::fs::remove_file(&enc_path).await;
    let _ = tokio::fs::remove_file(sibling(&enc_path, "-wal")).await;
    let _ = tokio::fs::remove_file(sibling(&enc_path, "-shm")).await;

    // Pre-create the encrypted target as a valid, keyed SQLCipher file via
    // SqliteConnectOptions (which normalises the path) so the raw `ATTACH`
    // below only has to *open* it — `ATTACH`-creating a new file by raw path
    // fails on Windows backslash paths (SQLITE_CANTOPEN).
    {
        let mut enc_conn = SqliteConnectOptions::from_str(enc_path.to_str().unwrap_or_default())?
            .pragma("key", key_pragma(db_key))
            .create_if_missing(true)
            .connect()
            .await?;
        sqlx::query("SELECT count(*) FROM sqlite_master")
            .execute(&mut enc_conn)
            .await?;
        enc_conn.close().await?;
    }

    // Open the plaintext DB (no key) and copy it into the encrypted file.
    let mut conn = SqliteConnectOptions::from_str(path.to_str().unwrap_or_default())?
        .create_if_missing(false)
        .connect()
        .await?;

    // Fold any WAL contents back into the main file so the backup is complete.
    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&mut conn)
        .await;

    // SQLite accepts forward slashes on every platform; backslashes in a raw
    // string literal can trip path opening on Windows.
    let enc_sql = sql_quote(&enc_path.to_string_lossy().replace('\\', "/"));
    // db_key is hex (no quotes) but quote defensively anyway.
    let key_sql = sql_quote(db_key);
    sqlx::query(&format!(
        "ATTACH DATABASE '{enc_sql}' AS encrypted KEY '{key_sql}'"
    ))
    .execute(&mut conn)
    .await?;
    sqlx::query("SELECT sqlcipher_export('encrypted')")
        .execute(&mut conn)
        .await?;
    sqlx::query("DETACH DATABASE encrypted")
        .execute(&mut conn)
        .await?;
    conn.close().await?;

    // The plaintext WAL/SHM (if any) is now stale — its data was checkpointed
    // into the main file above.  Remove it so it can't confuse the encrypted
    // database that takes the original's filename.
    let _ = tokio::fs::remove_file(sibling(path, "-wal")).await;
    let _ = tokio::fs::remove_file(sibling(path, "-shm")).await;

    // Swap files: original → backup, encrypted → original.
    let bak = sibling(path, ".pre-sqlcipher.bak");
    tokio::fs::rename(path, &bak).await?;
    tokio::fs::rename(&enc_path, path).await?;

    tracing::info!(
        db = ?path,
        backup = ?bak,
        "Database successfully encrypted; plaintext backup retained"
    );
    Ok(())
}

/// Build a sibling path by appending `suffix` to the full filename.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Escape single quotes for safe interpolation into a SQL string literal.
fn sql_quote(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{ConnectOptions, Connection};

    const SECRET: &str = "test-jwt-secret-at-least-32-characters-long!!";
    const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

    fn temp_db(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("sp_sqlcipher_{tag}_{nanos}.db"))
    }

    async fn open(path: &Path, key: Option<&str>) -> Result<sqlx::SqliteConnection, sqlx::Error> {
        let mut opts =
            SqliteConnectOptions::from_str(path.to_str().unwrap())?.create_if_missing(true);
        if let Some(k) = key {
            opts = opts.pragma("key", key_pragma(k));
        }
        opts.connect().await
    }

    #[tokio::test]
    async fn database_is_encrypted_at_rest() {
        let path = temp_db("enc");
        let key = derive_db_key(SECRET);
        let secret_value = "topsecret-home-address-86-nelson-blvd";

        {
            let mut conn = open(&path, Some(&key)).await.unwrap();
            sqlx::query("CREATE TABLE t(x TEXT)")
                .execute(&mut conn)
                .await
                .unwrap();
            sqlx::query("INSERT INTO t(x) VALUES(?1)")
                .bind(secret_value)
                .execute(&mut conn)
                .await
                .unwrap();
            conn.close().await.unwrap();
        }

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes.starts_with(SQLITE_MAGIC),
            "file must not be a plaintext SQLite database"
        );
        assert!(
            !bytes
                .windows(secret_value.len())
                .any(|w| w == secret_value.as_bytes()),
            "sensitive value must not appear in plaintext on disk"
        );

        // Reopening with the correct key returns the data.
        {
            let mut conn = open(&path, Some(&key)).await.unwrap();
            let (x,): (String,) = sqlx::query_as("SELECT x FROM t")
                .fetch_one(&mut conn)
                .await
                .unwrap();
            assert_eq!(x, secret_value);
            conn.close().await.unwrap();
        }

        // Reopening without the key cannot read the schema.
        let no_key = async {
            let mut conn = open(&path, None).await?;
            sqlx::query("SELECT x FROM t").fetch_one(&mut conn).await?;
            Ok::<_, sqlx::Error>(())
        }
        .await;
        assert!(no_key.is_err(), "must not be readable without the key");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn digit_first_key_opens() {
        // Regression: a hex key beginning with a digit must still open. sqlx
        // interpolates PRAGMA values verbatim, so without quoting SQLite reads
        // it as a malformed number ("unrecognized token") and boot fails.
        let mut seed = 0u32;
        let mut key = derive_db_key("seed0");
        while !key.starts_with(|c: char| c.is_ascii_digit()) {
            seed += 1;
            key = derive_db_key(&format!("seed{seed}"));
        }
        assert!(key.starts_with(|c: char| c.is_ascii_digit()));

        let path = temp_db("digitkey");
        {
            let mut conn = open(&path, Some(&key)).await.unwrap();
            sqlx::query("CREATE TABLE t(x TEXT)")
                .execute(&mut conn)
                .await
                .unwrap();
            conn.close().await.unwrap();
        }
        // Reopening with the digit-first key succeeds.
        let conn = open(&path, Some(&key)).await.unwrap();
        conn.close().await.unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn migrates_plaintext_db_to_encrypted() {
        let path = temp_db("migrate");
        let legacy_value = "legacy-plaintext-gps-40.7,-73.9";

        // Create a legacy *plaintext* database.
        {
            let mut conn = open(&path, None).await.unwrap();
            sqlx::query("CREATE TABLE t(x TEXT)")
                .execute(&mut conn)
                .await
                .unwrap();
            sqlx::query("INSERT INTO t(x) VALUES(?1)")
                .bind(legacy_value)
                .execute(&mut conn)
                .await
                .unwrap();
            conn.close().await.unwrap();
        }
        assert!(
            std::fs::read(&path).unwrap().starts_with(SQLITE_MAGIC),
            "precondition: db starts plaintext"
        );

        // Migrate.
        let key = derive_db_key(SECRET);
        ensure_encrypted(&path, &key).await.unwrap();

        // Now encrypted, and a plaintext backup was retained.
        let bytes = std::fs::read(&path).unwrap();
        assert!(!bytes.starts_with(SQLITE_MAGIC), "db must now be encrypted");
        let bak = sibling(&path, ".pre-sqlcipher.bak");
        assert!(bak.exists(), "plaintext backup must be retained");

        // Data survived the migration and is readable with the key.
        {
            let mut conn = open(&path, Some(&key)).await.unwrap();
            let (x,): (String,) = sqlx::query_as("SELECT x FROM t")
                .fetch_one(&mut conn)
                .await
                .unwrap();
            assert_eq!(x, legacy_value);
            conn.close().await.unwrap();
        }

        // ensure_encrypted is idempotent on an already-encrypted db.
        ensure_encrypted(&path, &key).await.unwrap();

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&bak);
    }
}
