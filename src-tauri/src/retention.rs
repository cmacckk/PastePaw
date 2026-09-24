//! Retention policy: how long clipboard history is kept, and how much of it.
//!
//! Clips saved to a folder are exempt from both limits, which is how Paste behaves:
//! anything pinned stays regardless of the history setting. `clips.content` and the
//! rows in `clip_images` / `clip_formats` are removed by cascade, but the image files
//! those rows point at are not, so they have to be deleted first.

use crate::clipboard::remove_full_image_file;
use sqlx::SqlitePool;

/// What a single prune pass removed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PruneReport {
    /// Clips dropped because they were older than the retention window.
    pub by_age: u64,
    /// Clips dropped because the history was over its item limit.
    pub by_count: u64,
}

impl PruneReport {
    pub fn total(&self) -> u64 {
        self.by_age + self.by_count
    }
}

/// How many clip uuids go into one statement.
///
/// SQLite bounds how many parameters a statement may take, so a long expiry list has
/// to be deleted in batches rather than in a single `IN (...)`.
const DELETE_BATCH: usize = 500;

/// Applies the retention policy.
///
/// A value of zero or less means "keep forever" for that dimension, so the default
/// of a fresh install (`auto_delete_days = 30`, `max_items = 1000`) prunes, while an
/// explicit zero disables it.
pub async fn prune(
    pool: &SqlitePool,
    auto_delete_days: i64,
    max_items: i64,
) -> Result<PruneReport, String> {
    let mut report = PruneReport::default();

    if auto_delete_days > 0 {
        let offset = format!("-{auto_delete_days} days");
        let expired: Vec<String> = sqlx::query_scalar(
            r#"SELECT uuid FROM clips
               WHERE folder_id IS NULL AND created_at < datetime('now', ?)"#,
        )
        .bind(&offset)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

        report.by_age = delete_clips(pool, &expired).await?;
    }

    if max_items > 0 {
        // Cheap guard. The query below materialises the surviving set, so it is only
        // worth running when the history is actually over its limit; this count is a
        // single pass over an existing index.
        let history_len: i64 =
            sqlx::query_scalar(r#"SELECT COUNT(*) FROM clips WHERE folder_id IS NULL"#)
                .fetch_one(pool)
                .await
                .map_err(|e| e.to_string())?;

        if history_len > max_items {
            let excess: Vec<String> = sqlx::query_scalar(
                r#"SELECT uuid FROM clips
                   WHERE folder_id IS NULL
                     AND uuid NOT IN (
                         SELECT uuid FROM clips
                         WHERE folder_id IS NULL
                         ORDER BY created_at DESC, id DESC
                         LIMIT ?
                     )"#,
            )
            .bind(max_items)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;

            report.by_count = delete_clips(pool, &excess).await?;
        }
    }

    Ok(report)
}

/// Deletes the given clips along with the image files they own.
///
/// The files go first: `clip_images` cascades away with the clip, so afterwards
/// nothing records where those files were and they would be orphaned on disk.
async fn delete_clips(pool: &SqlitePool, clip_uuids: &[String]) -> Result<u64, String> {
    if clip_uuids.is_empty() {
        return Ok(0);
    }

    let mut deleted = 0u64;
    for batch in clip_uuids.chunks(DELETE_BATCH) {
        let placeholders = vec!["?"; batch.len()].join(",");

        let path_sql =
            format!("SELECT file_path FROM clip_images WHERE clip_uuid IN ({placeholders})");
        let mut paths = sqlx::query_scalar::<_, Option<String>>(&path_sql);
        for uuid in batch {
            paths = paths.bind(uuid);
        }
        let paths = paths.fetch_all(pool).await.map_err(|e| e.to_string())?;
        for path in paths.into_iter().flatten() {
            if !path.is_empty() {
                remove_full_image_file(&path);
            }
        }

        let delete_sql = format!("DELETE FROM clips WHERE uuid IN ({placeholders})");
        let mut delete = sqlx::query(&delete_sql);
        for uuid in batch {
            delete = delete.bind(uuid);
        }
        deleted += delete
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?
            .rows_affected();
    }

    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;

    async fn test_db() -> Database {
        let db = Database::in_memory().await;
        db.migrate().await.expect("migrate");
        db
    }

    /// `age` is a SQLite datetime modifier such as `-40 days`.
    async fn insert_clip(db: &Database, uuid: &str, folder_id: Option<i64>, age: &str) {
        sqlx::query(
            r#"INSERT INTO clips (uuid, clip_type, content, content_hash, text_preview, folder_id, created_at)
               VALUES (?, 'text', x'', ?, 'preview', ?, datetime('now', ?))"#,
        )
        .bind(uuid)
        .bind(format!("hash-{uuid}"))
        .bind(folder_id)
        .bind(age)
        .execute(&db.pool)
        .await
        .expect("insert clip");
    }

    async fn clip_exists(db: &Database, uuid: &str) -> bool {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM clips WHERE uuid = ?")
            .bind(uuid)
            .fetch_one(&db.pool)
            .await
            .expect("count clip")
            > 0
    }

    async fn make_folder(db: &Database) -> i64 {
        sqlx::query_scalar::<_, i64>("INSERT INTO folders (name) VALUES ('saved') RETURNING id")
            .fetch_one(&db.pool)
            .await
            .expect("insert folder")
    }

    #[tokio::test]
    async fn history_older_than_the_window_is_removed() {
        let db = test_db().await;
        insert_clip(&db, "old", None, "-40 days").await;
        insert_clip(&db, "fresh", None, "-2 days").await;

        let report = prune(&db.pool, 30, 0).await.expect("prune");

        assert_eq!(report.by_age, 1);
        assert!(!clip_exists(&db, "old").await);
        assert!(clip_exists(&db, "fresh").await);
    }

    #[tokio::test]
    async fn clips_saved_to_a_folder_never_expire() {
        let db = test_db().await;
        let folder = make_folder(&db).await;
        insert_clip(&db, "pinned", Some(folder), "-400 days").await;
        insert_clip(&db, "history", None, "-400 days").await;

        let report = prune(&db.pool, 30, 0).await.expect("prune");

        assert_eq!(report.by_age, 1);
        assert!(
            clip_exists(&db, "pinned").await,
            "a folder clip is permanent"
        );
        assert!(!clip_exists(&db, "history").await);
    }

    #[tokio::test]
    async fn a_window_of_zero_keeps_everything() {
        let db = test_db().await;
        insert_clip(&db, "ancient", None, "-4000 days").await;

        let report = prune(&db.pool, 0, 0).await.expect("prune");

        assert_eq!(report, PruneReport::default());
        assert!(clip_exists(&db, "ancient").await);
    }

    #[tokio::test]
    async fn the_item_limit_drops_the_oldest_beyond_it() {
        let db = test_db().await;
        for index in 0..5 {
            insert_clip(
                &db,
                &format!("clip-{index}"),
                None,
                &format!("-{index} days"),
            )
            .await;
        }

        // The three most recent survive; clip-3 and clip-4 are the oldest.
        let report = prune(&db.pool, 0, 3).await.expect("prune");

        assert_eq!(report.by_count, 2);
        for index in 0..3 {
            assert!(
                clip_exists(&db, &format!("clip-{index}")).await,
                "clip-{index} should have survived"
            );
        }
        for index in 3..5 {
            assert!(
                !clip_exists(&db, &format!("clip-{index}")).await,
                "clip-{index} should have been dropped"
            );
        }
    }

    #[tokio::test]
    async fn the_item_limit_does_not_count_folder_clips() {
        let db = test_db().await;
        let folder = make_folder(&db).await;
        for index in 0..3 {
            insert_clip(&db, &format!("pinned-{index}"), Some(folder), "-1 days").await;
        }
        insert_clip(&db, "history", None, "-1 days").await;

        // Four clips exist, but only one of them is history.
        let report = prune(&db.pool, 0, 1).await.expect("prune");

        assert_eq!(report, PruneReport::default());
        assert!(clip_exists(&db, "history").await);
        assert!(clip_exists(&db, "pinned-0").await);
    }

    #[tokio::test]
    async fn an_item_limit_at_or_under_the_history_length_does_nothing() {
        let db = test_db().await;
        insert_clip(&db, "a", None, "-1 days").await;
        insert_clip(&db, "b", None, "-2 days").await;

        assert_eq!(
            prune(&db.pool, 0, 2).await.expect("prune"),
            PruneReport::default()
        );
        assert!(clip_exists(&db, "a").await);
        assert!(clip_exists(&db, "b").await);
    }

    /// The image files are deleted before the rows cascade, so a pruned image clip
    /// must not leave its file behind. A path that does not exist stands in for that
    /// here; the point is that the row is gone and the delete path was exercised.
    #[tokio::test]
    async fn pruning_a_clip_removes_its_format_and_image_rows() {
        let db = test_db().await;
        insert_clip(&db, "doomed", None, "-40 days").await;

        sqlx::query("INSERT INTO clip_formats (clip_uuid, format, content) VALUES (?, 'html', ?)")
            .bind("doomed")
            .bind(b"<b>x</b>".to_vec())
            .execute(&db.pool)
            .await
            .expect("insert format");
        sqlx::query(
            "INSERT INTO clip_images (clip_uuid, full_content, file_path, storage_kind)
             VALUES (?, x'', '/nonexistent/pastepaw-test.png', 'file')",
        )
        .bind("doomed")
        .execute(&db.pool)
        .await
        .expect("insert image row");

        prune(&db.pool, 30, 0).await.expect("prune");

        let formats: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM clip_formats")
            .fetch_one(&db.pool)
            .await
            .expect("count formats");
        let images: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM clip_images")
            .fetch_one(&db.pool)
            .await
            .expect("count images");

        assert_eq!(formats, 0);
        assert_eq!(images, 0);
    }

    #[tokio::test]
    async fn both_limits_apply_in_one_pass() {
        let db = test_db().await;
        insert_clip(&db, "expired", None, "-40 days").await;
        insert_clip(&db, "keep-1", None, "-2 days").await;
        insert_clip(&db, "keep-2", None, "-1 days").await;

        let report = prune(&db.pool, 30, 2).await.expect("prune");

        assert_eq!(report.by_age, 1);
        assert_eq!(report.by_count, 0);
        assert_eq!(report.total(), 1);
    }
}
