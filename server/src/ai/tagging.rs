//! Auto-tagging: applies AI-detected faces and objects as tags on photos.
//!
//! Face clusters get tags with the prefix `person:` (e.g., `person:John`).
//! Object detections get tags with the prefix `object:` (e.g., `object:cat`).
//!
//! Uses the existing `photo_tags` table so AI tags seamlessly integrate
//! with manual tags and the tag search system.

use sqlx::SqlitePool;

/// Apply a face cluster label as a tag to a photo.
///
/// Creates a tag with the `person:` prefix. If the cluster has no label
/// yet, uses `person:Unknown Face #<cluster_id>`.
pub async fn apply_face_tag(
    pool: &SqlitePool,
    user_id: &str,
    photo_id: &str,
    cluster_id: i64,
    label: Option<&str>,
) -> anyhow::Result<()> {
    let tag_name = match label {
        Some(name) if !name.is_empty() => format!("person:{}", name),
        _ => format!("person:Unknown Face #{}", cluster_id),
    };

    // Use INSERT OR IGNORE to avoid duplicates
    sqlx::query(
        "INSERT OR IGNORE INTO photo_tags (photo_id, user_id, tag) VALUES (?1, ?2, ?3)"
    )
    .bind(photo_id)
    .bind(user_id)
    .bind(&tag_name)
    .execute(pool)
    .await?;

    Ok(())
}

/// Apply an object detection as a tag to a photo.
///
/// Creates a tag with the `object:` prefix.
pub async fn apply_object_tag(
    pool: &SqlitePool,
    user_id: &str,
    photo_id: &str,
    class_name: &str,
) -> anyhow::Result<()> {
    let tag_name = format!("object:{}", class_name);

    sqlx::query(
        "INSERT OR IGNORE INTO photo_tags (photo_id, user_id, tag) VALUES (?1, ?2, ?3)"
    )
    .bind(photo_id)
    .bind(user_id)
    .bind(&tag_name)
    .execute(pool)
    .await?;

    Ok(())
}

/// Remove all AI-generated tags (person: and object:) for a photo.
///
/// Called before re-processing to avoid stale tags.
pub async fn clear_ai_tags(
    pool: &SqlitePool,
    user_id: &str,
    photo_id: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM photo_tags WHERE photo_id = ?1 AND user_id = ?2 AND (tag LIKE 'person:%' OR tag LIKE 'object:%')"
    )
    .bind(photo_id)
    .bind(user_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// When a face cluster is renamed, update all associated tags.
///
/// Changes `person:OldName` or `person:Unknown Face #N` to `person:NewName`
/// for all photos in the cluster.
pub async fn rename_cluster_tags(
    pool: &SqlitePool,
    user_id: &str,
    cluster_id: i64,
    new_label: &str,
) -> anyhow::Result<u64> {
    let new_tag = format!("person:{}", new_label);

    // Find all photos in this cluster
    let photo_ids: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT photo_id FROM face_detections WHERE cluster_id = ?1 AND user_id = ?2"
    )
    .bind(cluster_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    let mut updated = 0u64;
    for (photo_id,) in &photo_ids {
        // Remove old person tags for this photo that came from this cluster
        sqlx::query(
            "DELETE FROM photo_tags WHERE photo_id = ?1 AND user_id = ?2 AND tag LIKE 'person:%'"
        )
        .bind(photo_id)
        .bind(user_id)
        .execute(pool)
        .await?;

        // Re-apply tags for all clusters this photo belongs to
        let clusters: Vec<(i64, Option<String>)> = sqlx::query_as(
            "SELECT DISTINCT fc.id, fc.label FROM face_clusters fc \
             JOIN face_detections fd ON fd.cluster_id = fc.id \
             WHERE fd.photo_id = ?1 AND fd.user_id = ?2"
        )
        .bind(photo_id)
        .bind(user_id)
        .fetch_all(pool)
        .await?;

        for (cid, label) in &clusters {
            let tag = if *cid == cluster_id {
                new_tag.clone()
            } else {
                match label {
                    Some(l) if !l.is_empty() => format!("person:{}", l),
                    _ => format!("person:Unknown Face #{}", cid),
                }
            };
            sqlx::query(
                "INSERT OR IGNORE INTO photo_tags (photo_id, user_id, tag) VALUES (?1, ?2, ?3)"
            )
            .bind(photo_id)
            .bind(user_id)
            .bind(&tag)
            .execute(pool)
            .await?;
        }
        updated += 1;
    }

    Ok(updated)
}
