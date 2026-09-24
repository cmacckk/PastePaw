use sqlx::SqlitePool;

#[derive(Clone)]
pub struct Database {
    pub pool: SqlitePool,
}

impl Database {
    pub async fn new(db_path: &str) -> Self {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true);

        let pool = SqlitePool::connect_with(options).await.unwrap();

        Self { pool }
    }

    pub async fn migrate(&self) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                icon TEXT,
                color TEXT,
                is_system INTEGER DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS clips (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                uuid TEXT NOT NULL UNIQUE,
                clip_type TEXT NOT NULL,
                content BLOB NOT NULL,
                text_preview TEXT,
                content_hash TEXT NOT NULL,
                folder_id INTEGER REFERENCES folders(id),
                is_deleted INTEGER DEFAULT 0,
                is_thumbnail INTEGER NOT NULL DEFAULT 0,
                source_app TEXT,
                source_icon TEXT,
                metadata TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                last_accessed DATETIME DEFAULT CURRENT_TIMESTAMP
            )
        "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_clips_hash ON clips(content_hash);
        "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_clips_folder ON clips(folder_id);
        "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_clips_created ON clips(created_at);
        "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )
        "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS ignored_apps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                app_name TEXT NOT NULL UNIQUE
            )
        "#,
        )
        .execute(&self.pool)
        .await?;

        // Backward-compatible schema updates.
        add_column_if_missing(
            &self.pool,
            "ALTER TABLE clips ADD COLUMN is_thumbnail INTEGER NOT NULL DEFAULT 0",
        )
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS clip_images (
                clip_uuid TEXT PRIMARY KEY,
                full_content BLOB NOT NULL,
                file_path TEXT,
                file_size INTEGER,
                storage_kind TEXT NOT NULL DEFAULT 'db',
                mime_type TEXT NOT NULL DEFAULT 'image/png',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (clip_uuid) REFERENCES clips(uuid) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_clip_images_storage ON clip_images(storage_kind);
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Additional clipboard formats for a clip (CF_HTML, CF_RTF, CF_HDROP).
        // `clips.content` keeps the primary payload used for search, dedup and
        // previews; this table only holds the richer alternates that get written
        // back to the clipboard when pasting.
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS clip_formats (
                clip_uuid TEXT NOT NULL,
                format TEXT NOT NULL,
                content BLOB NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (clip_uuid, format),
                FOREIGN KEY (clip_uuid) REFERENCES clips(uuid) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

async fn add_column_if_missing(pool: &SqlitePool, sql: &str) -> Result<(), sqlx::Error> {
    match sqlx::query(sql).execute(pool).await {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            if msg.contains("duplicate column name") {
                Ok(())
            } else {
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory SQLite. `max_connections(1)` matters: each pool connection gets
    /// its own in-memory database, so a migration would otherwise land on a
    /// connection the test never queries.
    async fn test_db() -> Database {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory sqlite");
        Database { pool }
    }

    async fn insert_clip(db: &Database, uuid: &str) {
        sqlx::query(
            r#"INSERT INTO clips (uuid, clip_type, content, content_hash, text_preview)
               VALUES (?, 'text', x'', ?, 'preview')"#,
        )
        .bind(uuid)
        .bind(format!("hash-{uuid}"))
        .execute(&db.pool)
        .await
        .expect("insert clip");
    }

    #[tokio::test]
    async fn migrate_creates_the_clip_formats_table() {
        let db = test_db().await;
        db.migrate().await.expect("migrate");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='clip_formats'",
        )
        .fetch_one(&db.pool)
        .await
        .expect("query sqlite_master");

        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn migrate_is_idempotent() {
        let db = test_db().await;
        for _ in 0..3 {
            db.migrate().await.expect("repeated migrate");
        }
    }

    /// Relies on sqlx enabling `PRAGMA foreign_keys` by default. If this starts
    /// failing, the cascade is off and deleting a clip leaks its format rows.
    #[tokio::test]
    async fn clip_formats_are_removed_with_their_clip() {
        let db = test_db().await;
        db.migrate().await.expect("migrate");
        insert_clip(&db, "uuid-1").await;

        sqlx::query("INSERT INTO clip_formats (clip_uuid, format, content) VALUES (?, ?, ?)")
            .bind("uuid-1")
            .bind("html")
            .bind(b"<b>x</b>".to_vec())
            .execute(&db.pool)
            .await
            .expect("insert format");

        sqlx::query("DELETE FROM clips WHERE uuid = ?")
            .bind("uuid-1")
            .execute(&db.pool)
            .await
            .expect("delete clip");

        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM clip_formats")
            .fetch_one(&db.pool)
            .await
            .expect("count formats");

        assert_eq!(remaining, 0);
    }

    #[tokio::test]
    async fn clip_formats_are_unique_per_clip_and_format() {
        let db = test_db().await;
        db.migrate().await.expect("migrate");
        insert_clip(&db, "uuid-2").await;

        for _ in 0..2 {
            sqlx::query(
                "INSERT OR REPLACE INTO clip_formats (clip_uuid, format, content)
                 VALUES (?, 'rtf', ?)",
            )
            .bind("uuid-2")
            .bind(b"{\\rtf1}".to_vec())
            .execute(&db.pool)
            .await
            .expect("upsert format");
        }

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM clip_formats")
            .fetch_one(&db.pool)
            .await
            .expect("count formats");

        assert_eq!(count, 1);
    }
}
