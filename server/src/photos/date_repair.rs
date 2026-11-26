//! Batch repair of photo dates, with a dry run and an undo (TODO.md §4b P1b).
//!
//! - `POST /api/photos/dates/repair`       — `{dry_run?, items:[{photo_id, taken_at, expect_taken_at}]}`
//! - `POST /api/photos/dates/repair/undo`  — `{dry_run?, batch_id}`
//!
//! Why it exists: the Android client read only MediaStore `DATE_TAKEN`, which is
//! NULL for a downloaded file, so thousands of rows were stored at 1970-01-01.
//! The phone encrypts before upload and the server's `created_at` is the upload
//! time, so only the phone can supply the right date. It sends corrections here.
//!
//! This rewrites many rows the user can see, so it is built to be hard to misuse:
//! - `dry_run` defaults to **true**; the caller must send `false` to write.
//! - An empty list is refused, as is anything over [`MAX_ITEMS`].
//! - Compare-and-set: each item names the value it expects to replace. A row
//!   changed since the phone looked is skipped (`stale`), never overwritten.
//! - Another user's photo answers `not_found`, exactly like a missing one.
//! - Every applied change records its old values in `taken_at_repairs`
//!   (migration 043) under one batch id; `…/undo` restores them exactly.
//!
//! The write side touches only `photos`; `photo_change_log` follows through its
//! UPDATE trigger (migration 033), so delta-sync clients are nominated for free.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use sqlx::SqlitePool;

use crate::auth::middleware::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

/// Upper bound on items per call. One write transaction holds SQLite's single
/// write lock for its whole duration; 500 point updates is a few milliseconds,
/// a whole library in one call is not.
pub const MAX_ITEMS: usize = 500;

/// How far into the future a repaired date may be. A day absorbs clock skew
/// between phone and server; anything beyond is a client bug.
const FUTURE_SLACK_MS: i64 = 24 * 60 * 60 * 1000;

// ── Wire types ────────────────────────────────────────────────────────

/// Distinguishes an absent field (`None`) from an explicit `null`
/// (`Some(None)`). `expect_taken_at: null` is a real claim ("the column is
/// NULL"); leaving the field out is not a claim at all and is refused.
fn present<'de, D>(d: D) -> Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Debug, Deserialize)]
pub struct RepairItem {
    pub photo_id: String,
    pub taken_at: String,
    #[serde(default, deserialize_with = "present")]
    pub expect_taken_at: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RepairRequest {
    /// Absent or `null` ⇒ dry run. Only an explicit `false` writes.
    pub dry_run: Option<bool>,
    pub items: Vec<RepairItem>,
}

#[derive(Debug, Deserialize)]
pub struct UndoRequest {
    pub dry_run: Option<bool>,
    pub batch_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Would change (dry run) / changed (applied).
    Change,
    /// The row already holds this instant — nothing to write.
    Unchanged,
    /// The row no longer holds the value the caller expected — left alone.
    Stale,
    /// No such photo for this user (missing, or someone else's).
    NotFound,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub change: usize,
    pub unchanged: usize,
    pub stale: usize,
    pub not_found: usize,
}

impl Counts {
    fn add(&mut self, o: Outcome) {
        match o {
            Outcome::Change => self.change += 1,
            Outcome::Unchanged => self.unchanged += 1,
            Outcome::Stale => self.stale += 1,
            Outcome::NotFound => self.not_found += 1,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ItemResult {
    pub photo_id: String,
    pub outcome: Outcome,
    /// The value the row held when this call looked (`None` = NULL or not found).
    pub current_taken_at: Option<String>,
    /// The canonical value written (or that would be written).
    pub new_taken_at: String,
}

#[derive(Debug, Serialize)]
pub struct RepairResponse {
    pub dry_run: bool,
    /// Set only when an applied call changed at least one row. Pass it to
    /// `…/repair/undo` to reverse exactly this call.
    pub batch_id: Option<String>,
    pub counts: Counts,
    pub items: Vec<ItemResult>,
}

#[derive(Debug, Serialize)]
pub struct UndoItemResult {
    pub photo_id: String,
    pub outcome: Outcome,
    pub current_taken_at: Option<String>,
    pub restore_taken_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UndoResponse {
    pub dry_run: bool,
    pub batch_id: String,
    pub counts: Counts,
    pub items: Vec<UndoItemResult>,
}

// ── Validation (pure) ─────────────────────────────────────────────────

/// The one place the default lives: absent or `null` is a dry run. Only an
/// explicit `false` writes.
pub fn is_dry_run(flag: Option<bool>) -> bool {
    flag != Some(false)
}

/// What the caller claims the column holds right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Null,
    Instant(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidItem {
    pub photo_id: String,
    /// Canonical `YYYY-MM-DDTHH:MM:SS.mmmZ`, the format every `taken_at` sorts by.
    pub new_taken_at: String,
    pub new_ms: i64,
    pub expect: Expect,
}

fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|d| d.with_timezone(&Utc).timestamp_millis())
}

fn canonical(ms: i64) -> Option<String> {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// Check a whole request before anything is read or written. Any bad item
/// rejects the whole call: a half-applied batch the caller did not expect is
/// harder to reason about than a 400.
pub fn validate_items(items: &[RepairItem], now_ms: i64) -> Result<Vec<ValidItem>, String> {
    if items.is_empty() {
        return Err("items is empty — refusing a repair with nothing named".into());
    }
    if items.len() > MAX_ITEMS {
        return Err(format!(
            "{} items exceeds the limit of {MAX_ITEMS} per call",
            items.len()
        ));
    }
    let mut seen = std::collections::HashSet::with_capacity(items.len());
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let id = item.photo_id.trim();
        if id.is_empty() {
            return Err(format!("items[{i}].photo_id is empty"));
        }
        if !seen.insert(id.to_string()) {
            return Err(format!("items[{i}].photo_id {id} appears more than once"));
        }
        let new_ms = parse_rfc3339_ms(&item.taken_at).ok_or_else(|| {
            format!(
                "items[{i}].taken_at is not an RFC 3339 instant: {:?}",
                item.taken_at
            )
        })?;
        if new_ms <= 0 {
            return Err(format!(
                "items[{i}].taken_at {} is at or before 1970-01-01 — that is the value being repaired",
                item.taken_at
            ));
        }
        if new_ms > now_ms + FUTURE_SLACK_MS {
            return Err(format!(
                "items[{i}].taken_at {} is in the future",
                item.taken_at
            ));
        }
        let expect = match &item.expect_taken_at {
            None => {
                return Err(format!(
                    "items[{i}].expect_taken_at is required (null when the date is NULL)"
                ))
            }
            Some(None) => Expect::Null,
            Some(Some(s)) => Expect::Instant(parse_rfc3339_ms(s).ok_or_else(|| {
                format!("items[{i}].expect_taken_at is not an RFC 3339 instant: {s:?}")
            })?),
        };
        let new_taken_at =
            canonical(new_ms).ok_or_else(|| format!("items[{i}].taken_at is out of range"))?;
        out.push(ValidItem {
            photo_id: id.to_string(),
            new_taken_at,
            new_ms,
            expect,
        });
    }
    Ok(out)
}

/// A stored `taken_at` as an instant. Stored values come in several historical
/// formats (`.000Z`, `Z`, offsets), so they are normalised before parsing.
/// `Err(())` = the column holds something unparseable.
fn stored_ms(stored: Option<&str>) -> Option<Result<i64, ()>> {
    stored.map(|s| parse_rfc3339_ms(&super::utils::normalize_iso_timestamp(s)).ok_or(()))
}

/// Decide one item against the row as it stands. Order matters: a row that
/// already holds the new instant is `unchanged` whatever was expected, and an
/// unparseable stored value can never be proven to match, so it is `stale`
/// (fail closed).
pub fn classify(row_exists: bool, stored: Option<&str>, item: &ValidItem) -> Outcome {
    if !row_exists {
        return Outcome::NotFound;
    }
    let current = stored_ms(stored);
    if current == Some(Ok(item.new_ms)) {
        return Outcome::Unchanged;
    }
    let matches = match (item.expect, current) {
        (Expect::Null, None) => true,
        (Expect::Instant(want), Some(Ok(have))) => want == have,
        _ => false,
    };
    if matches {
        Outcome::Change
    } else {
        Outcome::Stale
    }
}

// ── Repair ────────────────────────────────────────────────────────────

type CurrentRow = (Option<String>, Option<i64>, Option<i64>, Option<String>);

const SELECT_CURRENT: &str =
    "SELECT taken_at, photo_year, photo_month, taken_at_offset FROM photos \
     WHERE id = ?1 AND user_id = ?2";

/// Dry run: read-only, against any pool. Applies nothing.
pub async fn plan(
    pool: &SqlitePool,
    user_id: &str,
    items: &[ValidItem],
) -> Result<RepairResponse, sqlx::Error> {
    let mut counts = Counts::default();
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        let row: Option<CurrentRow> = sqlx::query_as(SELECT_CURRENT)
            .bind(&item.photo_id)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
        let stored = row.as_ref().and_then(|r| r.0.clone());
        let outcome = classify(row.is_some(), stored.as_deref(), item);
        counts.add(outcome);
        results.push(ItemResult {
            photo_id: item.photo_id.clone(),
            outcome,
            current_taken_at: stored,
            new_taken_at: item.new_taken_at.clone(),
        });
    }
    Ok(RepairResponse {
        dry_run: true,
        batch_id: None,
        counts,
        items: results,
    })
}

/// Apply in ONE write transaction. Every row is re-read inside it — the dry
/// run the caller saw may be minutes old. Nothing is committed unless at least
/// one row changes, so an all-skipped call leaves no empty batch behind.
pub async fn apply(
    pool: &SqlitePool,
    user_id: &str,
    items: &[ValidItem],
) -> Result<RepairResponse, sqlx::Error> {
    let batch_id = uuid::Uuid::new_v4().to_string();
    let now = super::utils::utc_now_iso();
    let mut tx = crate::db::begin_write(pool).await?;
    let mut counts = Counts::default();
    let mut results = Vec::with_capacity(items.len());

    for item in items {
        let row: Option<CurrentRow> = sqlx::query_as(SELECT_CURRENT)
            .bind(&item.photo_id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
        let stored = row.as_ref().and_then(|r| r.0.clone());
        let outcome = classify(row.is_some(), stored.as_deref(), item);

        if let (Outcome::Change, Some((old_taken, old_year, old_month, old_offset))) =
            (outcome, row.as_ref())
        {
            sqlx::query(
                "INSERT INTO taken_at_repairs \
                 (batch_id, user_id, photo_id, old_taken_at, old_photo_year, old_photo_month, \
                  old_taken_at_offset, new_taken_at, repaired_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .bind(&batch_id)
            .bind(user_id)
            .bind(&item.photo_id)
            .bind(old_taken)
            .bind(old_year)
            .bind(old_month)
            .bind(old_offset)
            .bind(&item.new_taken_at)
            .bind(&now)
            .execute(&mut *tx)
            .await?;

            // The offset is cleared: a phone DATE_ADDED is a true UTC instant
            // with no capture zone. The old one is in the undo row.
            let done = sqlx::query(
                "UPDATE photos SET taken_at = ?1, \
                 photo_year = CAST(strftime('%Y', ?1) AS INTEGER), \
                 photo_month = CAST(strftime('%m', ?1) AS INTEGER), \
                 taken_at_offset = NULL \
                 WHERE id = ?2 AND user_id = ?3",
            )
            .bind(&item.new_taken_at)
            .bind(&item.photo_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
            if done.rows_affected() != 1 {
                // Inside the write lock the row was just read — this cannot
                // happen unless the schema changed under us. Refuse the batch.
                tracing::error!(
                    photo_id = %item.photo_id, batch_id = %batch_id,
                    affected = done.rows_affected(),
                    "date repair: UPDATE touched an unexpected row count; rolling back the batch",
                );
                return Err(sqlx::Error::RowNotFound);
            }
        }

        counts.add(outcome);
        results.push(ItemResult {
            photo_id: item.photo_id.clone(),
            outcome,
            current_taken_at: stored,
            new_taken_at: item.new_taken_at.clone(),
        });
    }

    let batch_id = if counts.change > 0 {
        tx.commit().await?;
        Some(batch_id)
    } else {
        tx.rollback().await?;
        None
    };
    Ok(RepairResponse {
        dry_run: false,
        batch_id,
        counts,
        items: results,
    })
}

// ── Undo ──────────────────────────────────────────────────────────────

type RepairRow = (
    i64,
    String,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<String>,
    String,
);

const SELECT_BATCH: &str =
    "SELECT id, photo_id, old_taken_at, old_photo_year, old_photo_month, old_taken_at_offset, \
     new_taken_at FROM taken_at_repairs \
     WHERE user_id = ?1 AND batch_id = ?2 AND undone_at IS NULL ORDER BY id";

fn undo_outcome(row_exists: bool, current: Option<&str>, repaired_to: &str) -> Outcome {
    if !row_exists {
        return Outcome::NotFound;
    }
    let want = parse_rfc3339_ms(repaired_to);
    match (stored_ms(current), want) {
        (Some(Ok(have)), Some(want)) if have == want => Outcome::Change,
        _ => Outcome::Stale,
    }
}

/// `Ok(None)` = no un-undone batch with this id for this user (⇒ 404).
pub async fn undo(
    pool: &SqlitePool,
    user_id: &str,
    batch_id: &str,
    dry_run: bool,
) -> Result<Option<UndoResponse>, sqlx::Error> {
    if dry_run {
        let rows: Vec<RepairRow> = sqlx::query_as(SELECT_BATCH)
            .bind(user_id)
            .bind(batch_id)
            .fetch_all(pool)
            .await?;
        if rows.is_empty() {
            return Ok(None);
        }
        let mut counts = Counts::default();
        let mut items = Vec::with_capacity(rows.len());
        for (_, photo_id, old_taken, _, _, _, new_taken) in rows {
            let cur: Option<(Option<String>,)> =
                sqlx::query_as("SELECT taken_at FROM photos WHERE id = ?1 AND user_id = ?2")
                    .bind(&photo_id)
                    .bind(user_id)
                    .fetch_optional(pool)
                    .await?;
            let current = cur.as_ref().and_then(|c| c.0.clone());
            let outcome = undo_outcome(cur.is_some(), current.as_deref(), &new_taken);
            counts.add(outcome);
            items.push(UndoItemResult {
                photo_id,
                outcome,
                current_taken_at: current,
                restore_taken_at: old_taken,
            });
        }
        return Ok(Some(UndoResponse {
            dry_run: true,
            batch_id: batch_id.to_string(),
            counts,
            items,
        }));
    }

    let mut tx = crate::db::begin_write(pool).await?;
    let rows: Vec<RepairRow> = sqlx::query_as(SELECT_BATCH)
        .bind(user_id)
        .bind(batch_id)
        .fetch_all(&mut *tx)
        .await?;
    if rows.is_empty() {
        tx.rollback().await?;
        return Ok(None);
    }
    let mut counts = Counts::default();
    let mut items = Vec::with_capacity(rows.len());
    for (_, photo_id, old_taken, old_year, old_month, old_offset, new_taken) in rows {
        let cur: Option<(Option<String>,)> =
            sqlx::query_as("SELECT taken_at FROM photos WHERE id = ?1 AND user_id = ?2")
                .bind(&photo_id)
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;
        let current = cur.as_ref().and_then(|c| c.0.clone());
        let outcome = undo_outcome(cur.is_some(), current.as_deref(), &new_taken);
        if outcome == Outcome::Change {
            sqlx::query(
                "UPDATE photos SET taken_at = ?1, photo_year = ?2, photo_month = ?3, \
                 taken_at_offset = ?4 WHERE id = ?5 AND user_id = ?6",
            )
            .bind(&old_taken)
            .bind(old_year)
            .bind(old_month)
            .bind(&old_offset)
            .bind(&photo_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        }
        counts.add(outcome);
        items.push(UndoItemResult {
            photo_id,
            outcome,
            current_taken_at: current,
            restore_taken_at: old_taken,
        });
    }
    // The whole batch is marked, stale rows included: they were edited after
    // the repair and the edit wins. A second undo of the same batch is a 404.
    sqlx::query(
        "UPDATE taken_at_repairs SET undone_at = ?1 \
         WHERE user_id = ?2 AND batch_id = ?3 AND undone_at IS NULL",
    )
    .bind(super::utils::utc_now_iso())
    .bind(user_id)
    .bind(batch_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(UndoResponse {
        dry_run: false,
        batch_id: batch_id.to_string(),
        counts,
        items,
    }))
}

// ── Handlers ──────────────────────────────────────────────────────────

/// `POST /api/photos/dates/repair`
pub async fn repair_dates(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<RepairRequest>,
) -> Result<Json<RepairResponse>, AppError> {
    let dry_run = is_dry_run(req.dry_run);
    let items = validate_items(&req.items, Utc::now().timestamp_millis()).map_err(|msg| {
        tracing::warn!(user_id = %auth.user_id, error = %msg, "date repair refused");
        AppError::BadRequest(msg)
    })?;
    tracing::info!(
        user_id = %auth.user_id, items = items.len(), dry_run,
        "date repair: start",
    );

    if dry_run {
        let resp = plan(&state.read_pool, &auth.user_id, &items).await?;
        tracing::info!(user_id = %auth.user_id, counts = ?resp.counts, "date repair: dry run");
        return Ok(Json(resp));
    }

    let resp = apply(&state.pool, &auth.user_id, &items).await?;
    if let Some(batch) = &resp.batch_id {
        state.summary_cache.invalidate(&auth.user_id);
        // One event for the batch: a client treats any "photo" event as
        // "sync now", and 500 per-photo sends would overflow the channel.
        state.emit_sync(&auth.user_id, "photo", batch);
        crate::audit::log(
            &state,
            crate::audit::AuditEvent::PhotoDateRepair,
            Some(&auth.user_id),
            &headers,
            Some(serde_json::json!({
                "action": "apply",
                "batch_id": batch,
                "counts": resp.counts,
            })),
        )
        .await;
        tracing::info!(
            user_id = %auth.user_id, batch_id = %batch, counts = ?resp.counts,
            "date repair: applied — reverse with POST /api/photos/dates/repair/undo {{\"batch_id\":\"{batch}\",\"dry_run\":false}}",
        );
    } else {
        tracing::info!(
            user_id = %auth.user_id, counts = ?resp.counts,
            "date repair: nothing to change, no batch written",
        );
    }
    Ok(Json(resp))
}

/// `POST /api/photos/dates/repair/undo`
pub async fn undo_repair(
    State(state): State<AppState>,
    auth: AuthUser,
    headers: HeaderMap,
    Json(req): Json<UndoRequest>,
) -> Result<Json<UndoResponse>, AppError> {
    let dry_run = is_dry_run(req.dry_run);
    let batch_id = req.batch_id.trim();
    if batch_id.is_empty() {
        tracing::warn!(user_id = %auth.user_id, "date repair undo refused: empty batch_id");
        return Err(AppError::BadRequest("batch_id is empty".into()));
    }
    let resp = match undo(&state.pool, &auth.user_id, batch_id, dry_run).await? {
        Some(r) => r,
        None => {
            tracing::warn!(
                user_id = %auth.user_id, batch_id = %batch_id,
                "date repair undo: no un-undone batch with this id for this user",
            );
            return Err(AppError::NotFound);
        }
    };
    if dry_run {
        tracing::info!(user_id = %auth.user_id, batch_id = %batch_id, counts = ?resp.counts,
            "date repair undo: dry run");
        return Ok(Json(resp));
    }
    state.summary_cache.invalidate(&auth.user_id);
    state.emit_sync(&auth.user_id, "photo", batch_id);
    crate::audit::log(
        &state,
        crate::audit::AuditEvent::PhotoDateRepair,
        Some(&auth.user_id),
        &headers,
        Some(serde_json::json!({
            "action": "undo",
            "batch_id": batch_id,
            "counts": resp.counts,
        })),
    )
    .await;
    tracing::info!(user_id = %auth.user_id, batch_id = %batch_id, counts = ?resp.counts,
        "date repair undo: applied");
    Ok(Json(resp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::path::PathBuf;
    use std::str::FromStr;

    const NOW_MS: i64 = 1_795_000_000_000; // 2026-11-17
    const EPOCH: &str = "1970-01-01T00:00:00.000Z";
    const DOWNLOADED: &str = "2024-03-05T10:20:30.400Z";

    fn item(id: &str, taken: &str, expect: Option<Option<&str>>) -> RepairItem {
        RepairItem {
            photo_id: id.into(),
            taken_at: taken.into(),
            expect_taken_at: expect.map(|e| e.map(str::to_string)),
        }
    }

    fn valid(id: &str, taken: &str, expect: Option<&str>) -> ValidItem {
        validate_items(&[item(id, taken, Some(expect))], NOW_MS)
            .unwrap()
            .remove(0)
    }

    // ── validation ────────────────────────────────────────────────

    #[test]
    fn an_empty_list_is_refused() {
        let err = validate_items(&[], NOW_MS).unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn more_than_the_limit_is_refused() {
        let many: Vec<_> = (0..=MAX_ITEMS)
            .map(|i| item(&format!("p{i}"), DOWNLOADED, Some(Some(EPOCH))))
            .collect();
        assert!(validate_items(&many, NOW_MS).unwrap_err().contains("limit"));
        assert_eq!(
            validate_items(&many[..MAX_ITEMS], NOW_MS).unwrap().len(),
            MAX_ITEMS
        );
    }

    #[test]
    fn a_duplicate_photo_id_is_refused() {
        let err = validate_items(
            &[
                item("p1", DOWNLOADED, Some(Some(EPOCH))),
                item(" p1 ", DOWNLOADED, Some(None)),
            ],
            NOW_MS,
        )
        .unwrap_err();
        assert!(err.contains("more than once"), "{err}");
    }

    #[test]
    fn epoch_pre_epoch_future_and_garbage_dates_are_refused() {
        for bad in [
            EPOCH,
            "1969-12-31T23:59:59Z",
            "2099-01-01T00:00:00Z",
            "yesterday",
            "",
        ] {
            assert!(
                validate_items(&[item("p1", bad, Some(Some(EPOCH)))], NOW_MS).is_err(),
                "{bad:?} should be refused",
            );
        }
    }

    #[test]
    fn a_missing_expect_is_refused_but_an_explicit_null_is_a_claim() {
        let err = validate_items(&[item("p1", DOWNLOADED, None)], NOW_MS).unwrap_err();
        assert!(err.contains("required"), "{err}");
        assert_eq!(valid("p1", DOWNLOADED, None).expect, Expect::Null);
    }

    #[test]
    fn the_wire_tells_absent_expect_from_null_expect() {
        let absent: RepairItem =
            serde_json::from_str(r#"{"photo_id":"p","taken_at":"x"}"#).unwrap();
        let null: RepairItem =
            serde_json::from_str(r#"{"photo_id":"p","taken_at":"x","expect_taken_at":null}"#)
                .unwrap();
        assert_eq!(absent.expect_taken_at, None);
        assert_eq!(null.expect_taken_at, Some(None));
    }

    #[test]
    fn only_an_explicit_false_writes() {
        for (body, dry) in [
            (r#"{"items":[]}"#, true),
            (r#"{"items":[],"dry_run":null}"#, true),
            (r#"{"items":[],"dry_run":true}"#, true),
            (r#"{"items":[],"dry_run":false}"#, false),
        ] {
            let r: RepairRequest = serde_json::from_str(body).unwrap();
            assert_eq!(is_dry_run(r.dry_run), dry, "{body}");
        }
        let u: UndoRequest = serde_json::from_str(r#"{"batch_id":"b"}"#).unwrap();
        assert!(is_dry_run(u.dry_run));
    }

    #[test]
    fn the_new_date_is_stored_in_the_canonical_sort_format() {
        assert_eq!(
            valid("p1", "2024-03-05T12:20:30.4+02:00", Some(EPOCH)).new_taken_at,
            DOWNLOADED
        );
        assert_eq!(
            valid("p1", "2024-03-05T10:20:30Z", Some(EPOCH)).new_taken_at,
            "2024-03-05T10:20:30.000Z"
        );
    }

    // ── classification ────────────────────────────────────────────

    #[test]
    fn classify_compares_instants_not_strings() {
        let it = valid("p1", DOWNLOADED, Some(EPOCH));
        assert_eq!(
            classify(true, Some("1970-01-01T00:00:00Z"), &it),
            Outcome::Change
        );
        assert_eq!(classify(true, Some(EPOCH), &it), Outcome::Change);
        assert_eq!(
            classify(true, Some("2024-03-05T10:20:30.400+00:00"), &it),
            Outcome::Unchanged
        );
    }

    #[test]
    fn classify_skips_a_row_changed_since_the_caller_looked() {
        let it = valid("p1", DOWNLOADED, Some(EPOCH));
        assert_eq!(
            classify(true, Some("2025-01-01T00:00:00.000Z"), &it),
            Outcome::Stale
        );
        assert_eq!(classify(true, None, &it), Outcome::Stale);
        // Unparseable stored values can never be proven to match: fail closed.
        assert_eq!(classify(true, Some("garbage"), &it), Outcome::Stale);
        assert_eq!(classify(false, Some(EPOCH), &it), Outcome::NotFound);
        let null_it = valid("p1", DOWNLOADED, None);
        assert_eq!(classify(true, None, &null_it), Outcome::Change);
        assert_eq!(classify(true, Some(EPOCH), &null_it), Outcome::Stale);
    }

    // ── against a real migrated database ──────────────────────────

    async fn pool(tag: &str) -> (SqlitePool, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sp_daterepair_{tag}_{nanos}.db"));
        let opts = SqliteConnectOptions::from_str(path.to_str().unwrap())
            .unwrap()
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(10))
            .foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
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

    async fn insert(
        pool: &SqlitePool,
        id: &str,
        user: &str,
        taken: Option<&str>,
        offset: Option<&str>,
    ) {
        sqlx::query(
            "INSERT INTO photos (id, user_id, filename, file_path, mime_type, media_type, \
             size_bytes, width, height, created_at, taken_at, taken_at_offset) \
             VALUES (?1, ?2, ?1, '', 'image/jpeg', 'photo', 1, 1, 1, '2026-08-05T00:00:00.000Z', ?3, ?4)",
        )
        .bind(id)
        .bind(user)
        .bind(taken)
        .bind(offset)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE photos SET photo_year = CAST(strftime('%Y', taken_at) AS INTEGER), \
             photo_month = CAST(strftime('%m', taken_at) AS INTEGER) WHERE id = ?1",
        )
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn row(pool: &SqlitePool, id: &str) -> CurrentRow {
        sqlx::query_as(
            "SELECT taken_at, photo_year, photo_month, taken_at_offset FROM photos WHERE id = ?1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn repairs(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM taken_at_repairs")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn seq(pool: &SqlitePool, id: &str) -> i64 {
        sqlx::query_scalar("SELECT seq FROM photo_change_log WHERE photo_id = ?1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_dry_run_reports_and_writes_nothing() {
        let (pool, path) = pool("dry").await;
        insert(&pool, "p1", "u1", Some(EPOCH), None).await;
        let before = (row(&pool, "p1").await, seq(&pool, "p1").await);

        let resp = plan(&pool, "u1", &[valid("p1", DOWNLOADED, Some(EPOCH))])
            .await
            .unwrap();
        assert_eq!(
            resp.counts,
            Counts {
                change: 1,
                ..Counts::default()
            }
        );
        assert!(resp.dry_run && resp.batch_id.is_none());

        assert_eq!((row(&pool, "p1").await, seq(&pool, "p1").await), before);
        assert_eq!(repairs(&pool).await, 0);
        close(pool, path).await;
    }

    #[tokio::test]
    async fn apply_moves_the_row_and_undo_puts_back_all_four_columns() {
        let (pool, path) = pool("apply_undo").await;
        insert(&pool, "p1", "u1", Some(EPOCH), Some("+09:00")).await;
        let original = row(&pool, "p1").await;
        let seq_before = seq(&pool, "p1").await;

        let resp = apply(&pool, "u1", &[valid("p1", DOWNLOADED, Some(EPOCH))])
            .await
            .unwrap();
        let batch = resp.batch_id.clone().expect("a changed row yields a batch");
        assert_eq!(resp.counts.change, 1);
        assert_eq!(
            row(&pool, "p1").await,
            (Some(DOWNLOADED.into()), Some(2024), Some(3), None),
        );
        assert!(
            seq(&pool, "p1").await > seq_before,
            "change_log must nominate the photo"
        );

        // Undo dry run changes nothing.
        let dry = undo(&pool, "u1", &batch, true).await.unwrap().unwrap();
        assert_eq!(dry.counts.change, 1);
        assert_eq!(row(&pool, "p1").await.0.as_deref(), Some(DOWNLOADED));

        let done = undo(&pool, "u1", &batch, false).await.unwrap().unwrap();
        assert_eq!(done.counts.change, 1);
        assert_eq!(row(&pool, "p1").await, original, "undo restores exactly");
        assert!(
            undo(&pool, "u1", &batch, false).await.unwrap().is_none(),
            "a second undo is a 404"
        );
        close(pool, path).await;
    }

    #[tokio::test]
    async fn another_users_photo_is_not_found_and_untouched() {
        let (pool, path) = pool("other_user").await;
        insert(&pool, "theirs", "u2", Some(EPOCH), None).await;
        let before = row(&pool, "theirs").await;

        let resp = apply(&pool, "u1", &[valid("theirs", DOWNLOADED, Some(EPOCH))])
            .await
            .unwrap();
        assert_eq!(
            resp.counts,
            Counts {
                not_found: 1,
                ..Counts::default()
            }
        );
        assert!(resp.batch_id.is_none());
        assert_eq!(row(&pool, "theirs").await, before);
        assert_eq!(
            repairs(&pool).await,
            0,
            "an all-skipped call leaves no batch"
        );
        close(pool, path).await;
    }

    #[tokio::test]
    async fn a_stale_row_is_skipped_while_the_rest_of_the_batch_applies() {
        let (pool, path) = pool("stale").await;
        insert(&pool, "fresh", "u1", Some(EPOCH), None).await;
        insert(
            &pool,
            "edited",
            "u1",
            Some("2025-06-01T00:00:00.000Z"),
            None,
        )
        .await;

        let resp = apply(
            &pool,
            "u1",
            &[
                valid("fresh", DOWNLOADED, Some(EPOCH)),
                valid("edited", DOWNLOADED, Some(EPOCH)),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            resp.counts,
            Counts {
                change: 1,
                stale: 1,
                ..Counts::default()
            }
        );
        assert_eq!(
            row(&pool, "edited").await.0.as_deref(),
            Some("2025-06-01T00:00:00.000Z")
        );
        assert_eq!(repairs(&pool).await, 1);
        close(pool, path).await;
    }

    #[tokio::test]
    async fn undo_leaves_a_row_edited_after_the_repair() {
        let (pool, path) = pool("undo_stale").await;
        insert(&pool, "p1", "u1", Some(EPOCH), None).await;
        let batch = apply(&pool, "u1", &[valid("p1", DOWNLOADED, Some(EPOCH))])
            .await
            .unwrap()
            .batch_id
            .unwrap();
        sqlx::query("UPDATE photos SET taken_at = '2023-01-01T00:00:00.000Z' WHERE id = 'p1'")
            .execute(&pool)
            .await
            .unwrap();

        let resp = undo(&pool, "u1", &batch, false).await.unwrap().unwrap();
        assert_eq!(
            resp.counts,
            Counts {
                stale: 1,
                ..Counts::default()
            }
        );
        assert_eq!(
            row(&pool, "p1").await.0.as_deref(),
            Some("2023-01-01T00:00:00.000Z")
        );
        close(pool, path).await;
    }

    #[tokio::test]
    async fn another_user_cannot_undo_my_batch() {
        let (pool, path) = pool("undo_other").await;
        insert(&pool, "p1", "u1", Some(EPOCH), None).await;
        let batch = apply(&pool, "u1", &[valid("p1", DOWNLOADED, Some(EPOCH))])
            .await
            .unwrap()
            .batch_id
            .unwrap();
        assert!(undo(&pool, "u2", &batch, false).await.unwrap().is_none());
        assert_eq!(row(&pool, "p1").await.0.as_deref(), Some(DOWNLOADED));
        close(pool, path).await;
    }

    /// The reverse quoted in `043_taken_at_repairs.sql`, actually run.
    #[tokio::test]
    async fn the_documented_reverse_drops_the_table() {
        let (pool, path) = pool("reverse").await;
        sqlx::query("DROP INDEX IF EXISTS idx_taken_at_repairs_user_batch")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DROP TABLE IF EXISTS taken_at_repairs")
            .execute(&pool)
            .await
            .unwrap();
        let left: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE '%taken_at_repairs%'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(left, 0);
        // The photos table is untouched by the reverse.
        insert(&pool, "p1", "u1", Some(DOWNLOADED), None).await;
        close(pool, path).await;
    }
}
