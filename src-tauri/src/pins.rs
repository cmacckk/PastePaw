//! The built-in folder that `P` saves a clip into.
//!
//! Folders double as pinboards here: a clip inside one is exempt from the retention
//! policy, which is what makes pinning mean "keep this". Reusing the folder table
//! rather than adding a separate flag is what keeps the two consistent — there is no
//! second notion of saved to drift out of sync.

use sqlx::SqlitePool;

/// Name stored for the built-in folder.
///
/// The window renders a localised label for system folders, so this value is never
/// shown; it only has to stay stable. A name in the database would otherwise have to
/// be rewritten whenever the interface language changed.
pub const PINNED_FOLDER_NAME: &str = "Pinned";

/// Returns the id of the built-in folder, creating it the first time it is needed.
pub async fn pinned_folder_id(pool: &SqlitePool) -> Result<i64, String> {
    if let Some(id) = find_pinned_folder(pool).await? {
        return Ok(id);
    }

    // `WHERE NOT EXISTS` keeps two concurrent callers from creating two folders. The
    // loser of that race gets no row back and simply re-reads the winner's.
    let inserted: Option<i64> = sqlx::query_scalar(
        r#"INSERT INTO folders (name, icon, color, is_system)
           SELECT ?, NULL, NULL, 1
           WHERE NOT EXISTS (SELECT 1 FROM folders WHERE is_system = 1)
           RETURNING id"#,
    )
    .bind(PINNED_FOLDER_NAME)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    match inserted {
        Some(id) => Ok(id),
        None => find_pinned_folder(pool)
            .await?
            .ok_or_else(|| "the built-in folder disappeared while being created".to_string()),
    }
}

async fn find_pinned_folder(pool: &SqlitePool) -> Result<Option<i64>, String> {
    sqlx::query_scalar::<_, i64>(
        r#"SELECT id FROM folders WHERE is_system = 1 ORDER BY id LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())
}

/// Saves a clip into the built-in folder, or takes it back out.
///
/// Returns the state after the toggle. A clip that lives in some other folder is
/// moved into the built-in one rather than rejected, because "pin this" should not
/// depend on where the clip happens to be filed.
pub async fn toggle_pin(pool: &SqlitePool, clip_uuid: &str) -> Result<bool, String> {
    // Resolved before touching the clip so a bad clip id cannot create the folder as
    // a side effect. Cheaper to reason about, and the folder is only wanted if the
    // clip actually exists.
    let pinned_id = pinned_folder_id(pool).await?;

    let current: Option<Option<i64>> =
        sqlx::query_scalar(r#"SELECT folder_id FROM clips WHERE uuid = ?"#)
            .bind(clip_uuid)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;

    let current = current.ok_or_else(|| format!("clip {clip_uuid} does not exist"))?;
    let was_pinned = current == Some(pinned_id);

    sqlx::query(r#"UPDATE clips SET folder_id = ? WHERE uuid = ?"#)
        .bind(if was_pinned { None } else { Some(pinned_id) })
        .bind(clip_uuid)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(!was_pinned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::retention;

    async fn test_db() -> Database {
        let db = Database::in_memory().await;
        db.migrate().await.expect("migrate");
        db
    }

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

    async fn folder_of(db: &Database, uuid: &str) -> Option<i64> {
        sqlx::query_scalar::<_, Option<i64>>("SELECT folder_id FROM clips WHERE uuid = ?")
            .bind(uuid)
            .fetch_one(&db.pool)
            .await
            .expect("read folder_id")
    }

    #[tokio::test]
    async fn the_builtin_folder_is_created_once_and_reused() {
        let db = test_db().await;

        let first = pinned_folder_id(&db.pool).await.expect("create");
        let second = pinned_folder_id(&db.pool).await.expect("reuse");
        assert_eq!(first, second);

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM folders WHERE is_system = 1")
            .fetch_one(&db.pool)
            .await
            .expect("count system folders");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn toggling_moves_a_clip_in_and_back_out() {
        let db = test_db().await;
        insert_clip(&db, "clip", None, "-1 days").await;

        assert!(toggle_pin(&db.pool, "clip").await.expect("pin"));
        assert_eq!(
            folder_of(&db, "clip").await,
            Some(pinned_folder_id(&db.pool).await.unwrap())
        );

        assert!(!toggle_pin(&db.pool, "clip").await.expect("unpin"));
        assert_eq!(folder_of(&db, "clip").await, None);
    }

    /// The point of pinning: a pinned clip has to outlive the retention window.
    #[tokio::test]
    async fn a_pinned_clip_survives_the_retention_policy() {
        let db = test_db().await;
        insert_clip(&db, "pinned", None, "-400 days").await;
        insert_clip(&db, "unpinned", None, "-400 days").await;

        toggle_pin(&db.pool, "pinned").await.expect("pin");

        let report = retention::prune(&db.pool, 30, 0).await.expect("prune");

        assert_eq!(report.by_age, 1);
        let survivors: Vec<String> = sqlx::query_scalar("SELECT uuid FROM clips")
            .fetch_all(&db.pool)
            .await
            .expect("list clips");
        assert_eq!(survivors, vec!["pinned".to_string()]);
    }

    #[tokio::test]
    async fn a_clip_in_another_folder_is_moved_into_the_builtin_one() {
        let db = test_db().await;
        let other: i64 =
            sqlx::query_scalar("INSERT INTO folders (name) VALUES ('work') RETURNING id")
                .fetch_one(&db.pool)
                .await
                .expect("insert folder");
        insert_clip(&db, "clip", Some(other), "-1 days").await;

        assert!(toggle_pin(&db.pool, "clip").await.expect("pin"));
        assert_eq!(
            folder_of(&db, "clip").await,
            Some(pinned_folder_id(&db.pool).await.unwrap())
        );
    }

    #[tokio::test]
    async fn toggling_an_unknown_clip_fails_instead_of_pinning_nothing() {
        let db = test_db().await;
        assert!(toggle_pin(&db.pool, "nope").await.is_err());
    }
}
