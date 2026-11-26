//! Refresh-token rotation that survives a lost response (TODO.md §4 N1a).
//!
//! Every refresh revokes the presented token and issues a successor, recording
//! the link in `refresh_tokens.replaced_by`. A replayed revoked token is then
//! one of two things, and the successor tells them apart:
//!
//! - **successor unrevoked** = it was never presented, so the client never got
//!   it: the rotation response was lost. Revoke the unused successor, issue a
//!   fresh one, relink. The client recovers instead of 401ing until re-login.
//! - **successor revoked** (presented, logged out, or killed by an earlier
//!   recovery) = two parties hold tokens from one chain: theft. Revoke every
//!   token the user has, as before.
//!
//! If an attacker replays a stolen token first, they get a pair — exactly what
//! a lost response looks like — but the legitimate holder's successor is now
//! revoked with no link, so their next refresh trips the theft path and ends
//! the attacker's session too. Detection is delayed by at most one refresh,
//! not lost.
//!
//! The decision and its writes run in one `BEGIN IMMEDIATE` transaction, so
//! concurrent replays serialise and the chain never holds two live tokens.

use chrono::Utc;
use sqlx::SqlitePool;

use super::tokens::insert_refresh_token;

/// What presenting a refresh token amounted to.
#[derive(Debug, PartialEq, Eq)]
pub enum Redeemed {
    /// A successor was issued. `recovered` = the presented token had already
    /// been rotated, but its successor was never used (lost response).
    Rotated {
        user_id: String,
        raw_refresh: String,
        recovered: bool,
    },
    /// A revoked token whose chain had moved on: every token of `user_id`
    /// is now revoked.
    Theft { user_id: String },
    /// No unexpired row has this hash (expired, purged, or never issued).
    NotFound,
}

/// Redeem the refresh token whose SHA-256 hash is `token_hash`.
pub async fn redeem(
    pool: &SqlitePool,
    token_hash: &str,
    ttl_days: i64,
) -> Result<Redeemed, sqlx::Error> {
    let now = Utc::now().to_rfc3339();
    let mut tx = crate::db::begin_write(pool).await?;

    let row = sqlx::query_as::<_, (String, String, bool, Option<String>)>(
        "SELECT id, user_id, revoked != 0, replaced_by FROM refresh_tokens \
         WHERE token_hash = ? AND expires_at > ?",
    )
    .bind(token_hash)
    .bind(&now)
    .fetch_optional(&mut *tx)
    .await?;

    let Some((token_id, user_id, revoked, replaced_by)) = row else {
        tx.rollback().await?;
        return Ok(Redeemed::NotFound);
    };

    let recovered = if !revoked {
        false
    } else {
        let successor_live = match replaced_by.as_deref() {
            Some(successor) => {
                sqlx::query_scalar::<_, bool>(
                    "SELECT revoked = 0 FROM refresh_tokens WHERE id = ? AND expires_at > ?",
                )
                .bind(successor)
                .bind(&now)
                .fetch_optional(&mut *tx)
                .await?
                    == Some(true)
            }
            None => false,
        };
        if !successor_live {
            sqlx::query("UPDATE refresh_tokens SET revoked = 1 WHERE user_id = ?")
                .bind(&user_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok(Redeemed::Theft { user_id });
        }
        // Lost response: the successor never reached anyone legitimate. Kill
        // it WITHOUT a link, so if it does surface it is treated as stolen.
        sqlx::query("UPDATE refresh_tokens SET revoked = 1 WHERE id = ?")
            .bind(replaced_by.as_deref())
            .execute(&mut *tx)
            .await?;
        true
    };

    let (new_id, raw_refresh) = insert_refresh_token(&mut *tx, &user_id, ttl_days).await?;
    sqlx::query("UPDATE refresh_tokens SET revoked = 1, replaced_by = ? WHERE id = ?")
        .bind(&new_id)
        .bind(&token_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    Ok(Redeemed::Rotated {
        user_id,
        raw_refresh,
        recovered,
    })
}

/// Delete refresh tokens past their expiry. Returns the number deleted.
///
/// Revoked-but-unexpired rows are deliberately KEPT: they are the chain
/// `redeem` walks to recognise a lost response, and the evidence that turns a
/// replay into theft detection instead of a silent "invalid". (Purging them
/// hourly is what stranded the phone in N1.) Cost: one row per rotation per
/// device, ≈720 per device over a 30-day TTL.
pub async fn purge_expired<'e, E>(executor: E, now: &str) -> Result<u64, sqlx::Error>
where
    E: sqlx::SqliteExecutor<'e>,
{
    let r = sqlx::query("DELETE FROM refresh_tokens WHERE expires_at < ?")
        .bind(now)
        .execute(executor)
        .await?;
    Ok(r.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::tokens::hash_token;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::path::PathBuf;
    use std::str::FromStr;

    const USER: &str = "user-1";
    const TTL_DAYS: i64 = 30;

    /// File-backed WAL pool with several connections, like production — an
    /// in-memory single-connection pool would serialise the concurrency test
    /// at the pool and prove nothing about the transaction.
    async fn pool(tag: &str) -> (SqlitePool, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sp_refresh_{tag}_{nanos}.db"));
        let opts = SqliteConnectOptions::from_str(path.to_str().unwrap())
            .unwrap()
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(10))
            // Bare token rows without the users graph.
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        (pool, path)
    }

    async fn close(pool: SqlitePool, path: PathBuf) {
        pool.close().await;
        for suffix in ["", "-wal", "-shm"] {
            let mut p = path.clone().into_os_string();
            p.push(suffix);
            if let Err(e) = std::fs::remove_file(&p) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("test cleanup: could not remove {p:?}: {e}");
                }
            }
        }
    }

    async fn login(pool: &SqlitePool) -> String {
        insert_refresh_token(pool, USER, TTL_DAYS).await.unwrap().1
    }

    async fn present(pool: &SqlitePool, raw: &str) -> Redeemed {
        redeem(pool, &hash_token(raw), TTL_DAYS).await.unwrap()
    }

    fn rotated(r: Redeemed) -> (String, bool) {
        match r {
            Redeemed::Rotated {
                raw_refresh,
                recovered,
                ..
            } => (raw_refresh, recovered),
            other => panic!("expected Rotated, got {other:?}"),
        }
    }

    async fn live_count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM refresh_tokens WHERE user_id = ? AND revoked = 0")
            .bind(USER)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn rotation_issues_successor_and_leaves_one_live_token() {
        let (pool, path) = pool("rotate").await;
        let t = login(&pool).await;
        let (s, recovered) = rotated(present(&pool, &t).await);
        assert!(!recovered);
        assert_ne!(s, t);
        assert_eq!(live_count(&pool).await, 1);
        close(pool, path).await;
    }

    /// The N1 repro: the phone never got `s`, and replays `t` (14.5 h later in
    /// the field — no grace window, only the chain decides).
    #[tokio::test]
    async fn replay_with_unused_successor_recovers_and_kills_the_successor() {
        let (pool, path) = pool("lost").await;
        let t = login(&pool).await;
        let (lost, _) = rotated(present(&pool, &t).await);

        let (n, recovered) = rotated(present(&pool, &t).await);
        assert!(
            recovered,
            "a replay with an unused successor is a lost response"
        );
        assert_eq!(
            live_count(&pool).await,
            1,
            "the unused successor must be revoked"
        );

        // The recovered token is the live one and rotates normally.
        let (_, recovered) = rotated(present(&pool, &n).await);
        assert!(!recovered);

        // The successor the recovery killed is stolen if it ever shows up.
        assert_eq!(
            present(&pool, &lost).await,
            Redeemed::Theft {
                user_id: USER.into()
            }
        );
        assert_eq!(live_count(&pool).await, 0);
        close(pool, path).await;
    }

    #[tokio::test]
    async fn replay_after_successor_was_used_is_theft() {
        let (pool, path) = pool("theft").await;
        let t = login(&pool).await;
        let (s, _) = rotated(present(&pool, &t).await);
        rotated(present(&pool, &s).await);

        assert_eq!(
            present(&pool, &t).await,
            Redeemed::Theft {
                user_id: USER.into()
            }
        );
        assert_eq!(live_count(&pool).await, 0, "theft revokes every token");
        close(pool, path).await;
    }

    /// Rows revoked before migration 042 (and logged-out tokens) have no
    /// successor link — replaying them must stay theft, never recover.
    #[tokio::test]
    async fn revoked_token_without_a_successor_link_is_theft() {
        let (pool, path) = pool("legacy").await;
        let t = login(&pool).await;
        login(&pool).await; // another device stays logged in
        sqlx::query("UPDATE refresh_tokens SET revoked = 1 WHERE token_hash = ?")
            .bind(hash_token(&t))
            .execute(&pool)
            .await
            .unwrap();

        assert!(matches!(present(&pool, &t).await, Redeemed::Theft { .. }));
        assert_eq!(live_count(&pool).await, 0);
        close(pool, path).await;
    }

    /// Password change / admin revoke kill every token of the user. A replay
    /// of an old rotated token must not resurrect the session through the
    /// lost-response path — the mass revoke leaves no live successor.
    #[tokio::test]
    async fn replay_after_mass_revoke_does_not_resurrect_the_session() {
        let (pool, path) = pool("massrevoke").await;
        let t = login(&pool).await;
        rotated(present(&pool, &t).await); // successor unused
        sqlx::query("UPDATE refresh_tokens SET revoked = 1 WHERE user_id = ?")
            .bind(USER)
            .execute(&pool)
            .await
            .unwrap();

        assert!(matches!(present(&pool, &t).await, Redeemed::Theft { .. }));
        assert_eq!(live_count(&pool).await, 0);
        close(pool, path).await;
    }

    #[tokio::test]
    async fn unknown_or_expired_token_is_not_found() {
        let (pool, path) = pool("unknown").await;
        assert_eq!(present(&pool, "never-issued").await, Redeemed::NotFound);

        let t = login(&pool).await;
        sqlx::query("UPDATE refresh_tokens SET expires_at = '2000-01-01T00:00:00+00:00'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(present(&pool, &t).await, Redeemed::NotFound);
        close(pool, path).await;
    }

    /// The Android storm: many queued 401s replay the same stale token at
    /// once. They must serialise — every one succeeds, and exactly one token
    /// is left live (the last issued), never two.
    #[tokio::test]
    async fn concurrent_replays_serialise_to_a_single_live_token() {
        let (pool, path) = pool("concurrent").await;
        let t = login(&pool).await;
        rotated(present(&pool, &t).await); // response lost

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let pool = pool.clone();
            let hash = hash_token(&t);
            tasks.push(tokio::spawn(
                async move { redeem(&pool, &hash, TTL_DAYS).await },
            ));
        }
        for task in tasks {
            let outcome = task
                .await
                .unwrap()
                .expect("a replay must not fail with BUSY");
            assert!(matches!(
                outcome,
                Redeemed::Rotated {
                    recovered: true,
                    ..
                }
            ));
        }
        assert_eq!(live_count(&pool).await, 1);
        close(pool, path).await;
    }

    #[tokio::test]
    async fn purge_keeps_revoked_rows_until_they_expire() {
        let (pool, path) = pool("purge").await;
        let t = login(&pool).await;
        rotated(present(&pool, &t).await); // t is now revoked, unexpired
        let expired = login(&pool).await;
        sqlx::query("UPDATE refresh_tokens SET expires_at = '2000-01-01T00:00:00+00:00' WHERE token_hash = ?")
            .bind(hash_token(&expired))
            .execute(&pool)
            .await
            .unwrap();

        let deleted = purge_expired(&pool, &Utc::now().to_rfc3339())
            .await
            .unwrap();
        assert_eq!(deleted, 1, "only the expired row goes");
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM refresh_tokens")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 2, "the revoked chain link survives the purge");
        close(pool, path).await;
    }
}
