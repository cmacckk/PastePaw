//! Folder export and import.
//!
//! The bundle is deliberately plain JSON with explicit fields rather than a map of
//! mixed encodings, because one of the acceptance criteria is that a person can read
//! it and hand-edit it before importing.
//!
//! Folders are referenced by name, not by id: ids only mean anything inside the
//! database that produced them.
//!
//! Images follow the agreed compromise: the bundle records the absolute path on the
//! exporting machine and nothing else, so an image clips up as a placeholder on a
//! different machine. Importing on the same machine still finds the file and brings
//! the image back.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;

use crate::clipboard_formats::{FORMAT_FILE, FORMAT_HTML, FORMAT_RTF};

/// Bundle format version. Import rejects anything it does not recognise rather than
/// guessing at an unknown shape.
pub const BUNDLE_VERSION: u32 = 1;

/// Marks a clip whose image could not be restored, so the window can say so instead
/// of showing a silently broken image.
pub const MISSING_IMAGE_KEY: &str = "missing_image";

#[derive(Debug, Serialize, Deserialize)]
pub struct Bundle {
    pub version: u32,
    pub app: String,
    pub exported_at: String,
    pub folders: Vec<BundleFolder>,
    pub clips: Vec<BundleClip>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleFolder {
    pub name: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub is_system: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleClip {
    pub clip_type: String,
    /// Text as stored. For a `file` clip this is the newline joined path list, and
    /// for an `image` clip it is empty.
    pub content: String,
    pub text_preview: String,
    /// Used to recognise a clip that is already present, which is what makes
    /// importing the same bundle twice a no-op.
    pub content_hash: String,
    /// Folder this clip belongs to, or `null` for plain history.
    pub folder: Option<String>,
    pub source_app: Option<String>,
    pub metadata: Option<String>,
    pub created_at: String,
    /// HTML fragment, as text.
    pub html: Option<String>,
    /// RTF as base64, because RTF bytes are not valid UTF-8 in general.
    pub rtf_base64: Option<String>,
    /// Absolute path on the machine that produced the bundle.
    pub image_path: Option<String>,
    /// A name the user gave this clip. Optional, so a bundle written before this
    /// existed still reads: the field is absent and deserialises to `None`.
    #[serde(default)]
    pub title: Option<String>,
}

/// What an import did.
#[derive(Debug, Default, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub folders_created: u64,
    pub folders_reused: u64,
    pub clips_imported: u64,
    /// Clips whose content hash was already present, and which were therefore left
    /// alone. A second import of the same bundle reports everything here.
    pub clips_skipped: u64,
    pub images_restored: u64,
    pub images_missing: u64,
}

/// A dated default file name for the save dialog, so repeated exports do not
/// silently overwrite each other.
pub fn suggested_file_name() -> String {
    format!(
        "pastepaw-folders-{}.json",
        chrono::Utc::now().format("%Y-%m-%d")
    )
}

#[derive(sqlx::FromRow)]
struct ExportClipRow {
    uuid: String,
    clip_type: String,
    content: Vec<u8>,
    text_preview: String,
    content_hash: String,
    source_app: Option<String>,
    metadata: Option<String>,
    title: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    folder_name: Option<String>,
    image_path: Option<String>,
}

/// Collects every clip that lives in a folder, together with the folder list.
///
/// Only folder clips are exported. Plain history is what the retention policy is
/// allowed to drop, so treating it as worth carrying between machines would
/// contradict that.
pub async fn export_bundle(pool: &SqlitePool) -> Result<Bundle, String> {
    let folders: Vec<(String, Option<String>, Option<String>, i64)> =
        sqlx::query_as(r#"SELECT name, icon, color, is_system FROM folders ORDER BY id"#)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;

    let rows: Vec<ExportClipRow> = sqlx::query_as(
        r#"SELECT c.uuid, c.clip_type, c.content, c.text_preview, c.content_hash,
                  c.source_app, c.metadata, c.title, c.created_at,
                  f.name AS folder_name, ci.file_path AS image_path
           FROM clips c
           LEFT JOIN folders f ON c.folder_id = f.id
           LEFT JOIN clip_images ci ON ci.clip_uuid = c.uuid
           WHERE c.is_deleted = 0 AND c.folder_id IS NOT NULL
           ORDER BY c.created_at, c.id"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Fetched in one query rather than joined, because a clip can have several
    // formats and a join would multiply its row.
    let format_rows: Vec<(String, String, Vec<u8>)> = sqlx::query_as(
        r#"SELECT cf.clip_uuid, cf.format, cf.content
           FROM clip_formats cf
           JOIN clips c ON c.uuid = cf.clip_uuid
           WHERE c.folder_id IS NOT NULL AND c.is_deleted = 0"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut formats: HashMap<String, HashMap<String, Vec<u8>>> = HashMap::new();
    for (uuid, format, content) in format_rows {
        formats.entry(uuid).or_default().insert(format, content);
    }

    let clips = rows
        .into_iter()
        .map(|row| {
            let mut own = formats.remove(&row.uuid).unwrap_or_default();
            BundleClip {
                clip_type: row.clip_type,
                content: String::from_utf8_lossy(&row.content).into_owned(),
                text_preview: row.text_preview,
                content_hash: row.content_hash,
                folder: row.folder_name,
                source_app: row.source_app,
                metadata: row.metadata,
                created_at: row.created_at.to_rfc3339(),
                html: own
                    .remove(FORMAT_HTML)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
                rtf_base64: own.remove(FORMAT_RTF).map(|bytes| BASE64.encode(bytes)),
                image_path: row.image_path.filter(|path| !path.is_empty()),
                title: row.title,
            }
        })
        .collect();

    Ok(Bundle {
        version: BUNDLE_VERSION,
        app: "PastePaw".to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        folders: folders
            .into_iter()
            .map(|(name, icon, color, is_system)| BundleFolder {
                name,
                icon,
                color,
                is_system: is_system != 0,
            })
            .collect(),
        clips,
    })
}

/// Parses a bundle and applies it.
pub async fn import_bundle_json(pool: &SqlitePool, json: &str) -> Result<ImportReport, String> {
    let bundle: Bundle =
        serde_json::from_str(json).map_err(|e| format!("not a valid bundle: {e}"))?;
    import_bundle(pool, &bundle).await
}

/// Applies a bundle to the database.
///
/// Folders are matched by name and merged rather than duplicated, and a clip whose
/// content hash is already present is left alone. Those two rules are what make
/// importing the same bundle twice equivalent to importing it once.
pub async fn import_bundle(pool: &SqlitePool, bundle: &Bundle) -> Result<ImportReport, String> {
    if bundle.version != BUNDLE_VERSION {
        return Err(format!(
            "bundle version {} is not supported (this build reads version {BUNDLE_VERSION})",
            bundle.version
        ));
    }

    let mut report = ImportReport::default();
    let mut transaction = pool.begin().await.map_err(|e| e.to_string())?;

    // Folders first, so clips can be filed as they are inserted.
    let mut folder_ids: HashMap<String, i64> = HashMap::new();
    for folder in &bundle.folders {
        if let Some(id) = find_folder_by_name(&mut transaction, &folder.name).await? {
            folder_ids.insert(folder.name.clone(), id);
            report.folders_reused += 1;
            continue;
        }

        let id: i64 = sqlx::query_scalar(
            r#"INSERT INTO folders (name, icon, color, is_system)
               VALUES (?, ?, ?, ?) RETURNING id"#,
        )
        .bind(&folder.name)
        .bind(&folder.icon)
        .bind(&folder.color)
        .bind(i64::from(folder.is_system))
        .fetch_one(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;

        folder_ids.insert(folder.name.clone(), id);
        report.folders_created += 1;
    }

    for clip in &bundle.clips {
        let already_present: Option<String> =
            sqlx::query_scalar(r#"SELECT uuid FROM clips WHERE content_hash = ?"#)
                .bind(&clip.content_hash)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|e| e.to_string())?;

        if already_present.is_some() {
            report.clips_skipped += 1;
            continue;
        }

        let uuid = uuid::Uuid::new_v4().to_string();
        let folder_id = clip
            .folder
            .as_ref()
            .and_then(|name| folder_ids.get(name).copied());

        // The image is resolved before the row is written, because whether it could
        // be read decides what the clip's metadata has to say.
        let restored_image = match (&clip.clip_type[..], clip.image_path.as_deref()) {
            ("image", Some(path)) => crate::clipboard::read_full_image_file(path).ok(),
            _ => None,
        };

        let metadata = match (clip.clip_type.as_str(), restored_image.is_some()) {
            ("image", true) => clip.metadata.clone(),
            ("image", false) => missing_image_metadata(clip),
            _ => clip.metadata.clone(),
        };

        let created_at = chrono::DateTime::parse_from_rfc3339(&clip.created_at)
            .map(|parsed| parsed.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| {
                log::warn!(
                    "Import: clip {} has an unreadable created_at, using now",
                    clip.content_hash
                );
                chrono::Utc::now()
            });

        sqlx::query(
            r#"INSERT INTO clips (uuid, clip_type, content, text_preview, content_hash,
                                  folder_id, is_deleted, is_thumbnail, source_app, metadata,
                                  title, created_at, last_accessed)
               VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?, ?, ?, ?)"#,
        )
        .bind(&uuid)
        .bind(&clip.clip_type)
        .bind(clip.content.as_bytes())
        .bind(&clip.text_preview)
        .bind(&clip.content_hash)
        .bind(folder_id)
        .bind(&clip.source_app)
        .bind(&metadata)
        .bind(crate::commands::normalise_title(clip.title.clone()))
        .bind(created_at)
        .bind(created_at)
        .execute(&mut *transaction)
        .await
        .map_err(|e| e.to_string())?;

        if clip.clip_type == "image" {
            match &restored_image {
                Some(bytes) => {
                    let file_path = crate::clipboard::persist_full_image_file(&uuid, bytes)
                        .map_err(|e| format!("could not store the imported image: {e}"))?;
                    sqlx::query(
                        r#"INSERT OR REPLACE INTO clip_images
                               (clip_uuid, full_content, file_path, file_size, storage_kind, mime_type, created_at)
                           VALUES (?, x'', ?, ?, 'file', 'image/png', CURRENT_TIMESTAMP)"#,
                    )
                    .bind(&uuid)
                    .bind(&file_path)
                    .bind(bytes.len() as i64)
                    .execute(&mut *transaction)
                    .await
                    .map_err(|e| e.to_string())?;
                    report.images_restored += 1;
                }
                None => report.images_missing += 1,
            }
        }

        for (format, content) in stored_formats_for(clip) {
            sqlx::query(
                r#"INSERT OR REPLACE INTO clip_formats (clip_uuid, format, content)
                   VALUES (?, ?, ?)"#,
            )
            .bind(&uuid)
            .bind(format)
            .bind(content)
            .execute(&mut *transaction)
            .await
            .map_err(|e| e.to_string())?;
        }

        report.clips_imported += 1;
    }

    transaction.commit().await.map_err(|e| e.to_string())?;

    Ok(report)
}

/// Rebuilds the rows that go into `clip_formats` for an imported clip.
///
/// A file clip's list is derived from its newline joined content rather than being
/// carried separately, so the bundle does not hold the same paths twice.
fn stored_formats_for(clip: &BundleClip) -> Vec<(&'static str, Vec<u8>)> {
    let mut formats: Vec<(&'static str, Vec<u8>)> = Vec::new();

    if clip.clip_type == "file" {
        let paths: Vec<String> = clip
            .content
            .split('\n')
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .collect();

        if !paths.is_empty() {
            match serde_json::to_vec(&paths) {
                Ok(payload) => formats.push((FORMAT_FILE, payload)),
                Err(e) => log::error!("Import: could not encode a file list: {e}"),
            }
        }
    }

    if let Some(html) = &clip.html {
        formats.push((FORMAT_HTML, html.as_bytes().to_vec()));
    }

    if let Some(encoded) = &clip.rtf_base64 {
        match BASE64.decode(encoded) {
            Ok(bytes) => formats.push((FORMAT_RTF, bytes)),
            Err(e) => log::error!("Import: could not decode RTF for a clip: {e}"),
        }
    }

    formats
}

/// Describes a clip whose image could not be found, so the window can say so.
fn missing_image_metadata(clip: &BundleClip) -> Option<String> {
    let mut value = match clip.metadata.as_deref() {
        Some(existing) => serde_json::from_str::<serde_json::Value>(existing)
            .unwrap_or_else(|_| serde_json::json!({})),
        None => serde_json::json!({}),
    };

    if let Some(object) = value.as_object_mut() {
        object.insert(MISSING_IMAGE_KEY.to_string(), serde_json::json!(true));
        object.insert(
            "original_path".to_string(),
            serde_json::json!(clip.image_path),
        );
    }

    Some(value.to_string())
}

async fn find_folder_by_name(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    name: &str,
) -> Result<Option<i64>, String> {
    sqlx::query_scalar::<_, i64>(r#"SELECT id FROM folders WHERE name = ? ORDER BY id LIMIT 1"#)
        .bind(name)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard_formats::{FORMAT_HTML, FORMAT_RTF};
    use crate::database::Database;

    async fn test_db() -> Database {
        let db = Database::in_memory().await;
        db.migrate().await.expect("migrate");
        db
    }

    async fn make_folder(db: &Database, name: &str) -> i64 {
        sqlx::query_scalar("INSERT INTO folders (name) VALUES (?) RETURNING id")
            .bind(name)
            .fetch_one(&db.pool)
            .await
            .expect("insert folder")
    }

    async fn insert_clip(
        db: &Database,
        uuid: &str,
        clip_type: &str,
        content: &str,
        folder: Option<i64>,
    ) {
        sqlx::query(
            r#"INSERT INTO clips (uuid, clip_type, content, content_hash, text_preview, folder_id,
                                  created_at, last_accessed)
               VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)"#,
        )
        .bind(uuid)
        .bind(clip_type)
        .bind(content.as_bytes())
        .bind(format!("hash-{uuid}"))
        .bind(content.chars().take(20).collect::<String>())
        .bind(folder)
        .execute(&db.pool)
        .await
        .expect("insert clip");
    }

    async fn clipboard_clip_count(db: &Database) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM clips WHERE folder_id IS NOT NULL")
            .fetch_one(&db.pool)
            .await
            .expect("count clips")
    }

    #[tokio::test]
    async fn export_carries_folder_clips_and_leaves_history_behind() {
        let db = test_db().await;
        let folder = make_folder(&db, "work").await;
        insert_clip(&db, "filed", "text", "kept", Some(folder)).await;
        insert_clip(&db, "loose", "text", "dropped", None).await;

        let bundle = export_bundle(&db.pool).await.expect("export");

        assert_eq!(bundle.version, BUNDLE_VERSION);
        assert_eq!(bundle.clips.len(), 1);
        assert_eq!(bundle.clips[0].content, "kept");
        assert_eq!(bundle.clips[0].folder.as_deref(), Some("work"));
    }

    #[tokio::test]
    async fn a_bundle_survives_a_round_trip() {
        let source = test_db().await;
        let folder = make_folder(&source, "work").await;
        insert_clip(&source, "text-clip", "text", "hello", Some(folder)).await;
        sqlx::query("INSERT INTO clip_formats (clip_uuid, format, content) VALUES (?, ?, ?)")
            .bind("text-clip")
            .bind(FORMAT_HTML)
            .bind(b"<b>hello</b>".to_vec())
            .execute(&source.pool)
            .await
            .expect("insert html");
        sqlx::query("INSERT INTO clip_formats (clip_uuid, format, content) VALUES (?, ?, ?)")
            .bind("text-clip")
            .bind(FORMAT_RTF)
            .bind(b"{\\rtf1\\ansi}".to_vec())
            .execute(&source.pool)
            .await
            .expect("insert rtf");
        insert_clip(
            &source,
            "file-clip",
            "file",
            "C:\\a.txt\nC:\\b.txt",
            Some(folder),
        )
        .await;

        let json = serde_json::to_string(&export_bundle(&source.pool).await.expect("export"))
            .expect("serialize");

        let target = test_db().await;
        let report = import_bundle_json(&target.pool, &json)
            .await
            .expect("import");

        assert_eq!(report.folders_created, 1);
        assert_eq!(report.clips_imported, 2);
        assert_eq!(clipboard_clip_count(&target).await, 2);

        let html: Vec<u8> =
            sqlx::query_scalar("SELECT content FROM clip_formats WHERE format = ? LIMIT 1")
                .bind(FORMAT_HTML)
                .fetch_one(&target.pool)
                .await
                .expect("read html");
        assert_eq!(html, b"<b>hello</b>");

        let rtf: Vec<u8> =
            sqlx::query_scalar("SELECT content FROM clip_formats WHERE format = ? LIMIT 1")
                .bind(FORMAT_RTF)
                .fetch_one(&target.pool)
                .await
                .expect("read rtf");
        assert_eq!(rtf, b"{\\rtf1\\ansi}");

        // The file clip's path list has to come back as a stored format, otherwise
        // pasting it would find nothing.
        let file_list: Vec<u8> =
            sqlx::query_scalar("SELECT content FROM clip_formats WHERE format = ? LIMIT 1")
                .bind(FORMAT_FILE)
                .fetch_one(&target.pool)
                .await
                .expect("read file list");
        let paths: Vec<String> = serde_json::from_slice(&file_list).expect("decode file list");
        assert_eq!(
            paths,
            vec!["C:\\a.txt".to_string(), "C:\\b.txt".to_string()]
        );
    }

    /// The accepted criterion: importing the same bundle twice equals importing it once.
    #[tokio::test]
    async fn importing_twice_changes_nothing() {
        let source = test_db().await;
        let folder = make_folder(&source, "work").await;
        insert_clip(&source, "clip", "text", "hello", Some(folder)).await;
        let bundle = export_bundle(&source.pool).await.expect("export");

        let target = test_db().await;
        let first = import_bundle(&target.pool, &bundle).await.expect("first");
        let second = import_bundle(&target.pool, &bundle).await.expect("second");

        assert_eq!(first.clips_imported, 1);
        assert_eq!(second.clips_imported, 0);
        assert_eq!(second.clips_skipped, 1);
        assert_eq!(second.folders_created, 0);
        assert_eq!(second.folders_reused, 1);
        assert_eq!(clipboard_clip_count(&target).await, 1);
    }

    #[tokio::test]
    async fn a_folder_that_already_exists_is_merged_not_duplicated() {
        let source = test_db().await;
        let folder = make_folder(&source, "work").await;
        insert_clip(&source, "clip", "text", "hello", Some(folder)).await;
        let bundle = export_bundle(&source.pool).await.expect("export");

        let target = test_db().await;
        make_folder(&target, "work").await;

        let report = import_bundle(&target.pool, &bundle).await.expect("import");

        assert_eq!(report.folders_created, 0);
        assert_eq!(report.folders_reused, 1);
        let folders: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM folders WHERE name = 'work'")
            .fetch_one(&target.pool)
            .await
            .expect("count folders");
        assert_eq!(folders, 1, "the name must not be duplicated");
    }

    /// The agreed compromise: an image whose file is gone is still recorded, and
    /// marked so the window can say what happened.
    #[tokio::test]
    async fn an_unreadable_image_is_kept_and_marked_missing() {
        let source = test_db().await;
        let folder = make_folder(&source, "shots").await;
        insert_clip(&source, "img", "image", "", Some(folder)).await;
        sqlx::query(
            r#"INSERT INTO clip_images (clip_uuid, full_content, file_path, storage_kind)
               VALUES ('img', x'', '/nonexistent/pastepaw-import-test.png', 'file')"#,
        )
        .execute(&source.pool)
        .await
        .expect("insert image row");

        let bundle = export_bundle(&source.pool).await.expect("export");
        assert_eq!(
            bundle.clips[0].image_path.as_deref(),
            Some("/nonexistent/pastepaw-import-test.png")
        );

        let target = test_db().await;
        let report = import_bundle(&target.pool, &bundle).await.expect("import");

        assert_eq!(report.clips_imported, 1, "the entry is kept, not dropped");
        assert_eq!(report.images_missing, 1);
        assert_eq!(report.images_restored, 0);

        let metadata: Option<String> =
            sqlx::query_scalar("SELECT metadata FROM clips WHERE folder_id IS NOT NULL LIMIT 1")
                .fetch_one(&target.pool)
                .await
                .expect("read metadata");
        let metadata = metadata.expect("metadata");
        let parsed: serde_json::Value = serde_json::from_str(&metadata).expect("parse metadata");
        assert_eq!(parsed[MISSING_IMAGE_KEY], serde_json::json!(true));
    }

    /// The accepted criterion is that the bundle can be read and edited by hand. This
    /// edits one the way a person would in a text editor and imports the result.
    #[tokio::test]
    async fn a_hand_edited_bundle_imports() {
        let source = test_db().await;
        let folder = make_folder(&source, "work").await;
        insert_clip(&source, "clip", "text", "original", Some(folder)).await;

        let json =
            serde_json::to_string_pretty(&export_bundle(&source.pool).await.expect("export"))
                .expect("serialize");
        assert!(
            json.contains("\n  "),
            "the bundle has to be pretty printed to be hand editable"
        );

        let mut value: serde_json::Value = serde_json::from_str(&json).expect("parse the bundle");
        value["clips"][0]["content"] = serde_json::json!("edited by hand");
        value["clips"][0]["content_hash"] = serde_json::json!("hand-edited-hash");
        value["folders"]
            .as_array_mut()
            .expect("folders array")
            .push(serde_json::json!({
                "name": "added by hand",
                "icon": null,
                "color": null,
                "is_system": false
            }));

        let target = test_db().await;
        let report = import_bundle_json(&target.pool, &value.to_string())
            .await
            .expect("import");

        assert_eq!(report.clips_imported, 1);
        assert_eq!(report.folders_created, 2);

        let content: Vec<u8> = sqlx::query_scalar("SELECT content FROM clips LIMIT 1")
            .fetch_one(&target.pool)
            .await
            .expect("read content");
        assert_eq!(String::from_utf8_lossy(&content), "edited by hand");

        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM folders ORDER BY name")
            .fetch_all(&target.pool)
            .await
            .expect("read folders");
        assert_eq!(names, vec!["added by hand".to_string(), "work".to_string()]);
    }

    /// One accepted criterion carries a budget: a large export has to finish inside
    /// five seconds. The bound is deliberately loose so it does not flake on a busy
    /// machine, while still failing if the export ever becomes quadratic in the number
    /// of clips.
    #[tokio::test]
    async fn a_large_export_stays_inside_the_five_second_budget() {
        let db = test_db().await;
        let folder = make_folder(&db, "bulk").await;

        for index in 0..1000 {
            insert_clip(
                &db,
                &format!("bulk-{index}"),
                "text",
                "payload",
                Some(folder),
            )
            .await;
        }
        for index in 0..100 {
            let uuid = format!("shot-{index}");
            insert_clip(&db, &uuid, "image", "", Some(folder)).await;
            sqlx::query(
                r#"INSERT INTO clip_images (clip_uuid, full_content, file_path, storage_kind)
                   VALUES (?, x'', ?, 'file')"#,
            )
            .bind(&uuid)
            .bind(format!("/nonexistent/shot-{index}.png"))
            .execute(&db.pool)
            .await
            .expect("insert image row");
        }

        let started = std::time::Instant::now();
        let bundle = export_bundle(&db.pool).await.expect("export");
        let elapsed = started.elapsed();

        println!("exported {} clips in {elapsed:?}", bundle.clips.len());
        assert_eq!(bundle.clips.len(), 1100);
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "exporting 1100 clips took {elapsed:?}, the budget is 5s"
        );
    }

    #[tokio::test]
    async fn an_unsupported_version_is_rejected() {
        let db = test_db().await;
        let json = serde_json::json!({
            "version": BUNDLE_VERSION + 1,
            "app": "PastePaw",
            "exported_at": "2026-01-01T00:00:00Z",
            "folders": [],
            "clips": []
        })
        .to_string();

        let error = import_bundle_json(&db.pool, &json)
            .await
            .expect_err("must fail");
        assert!(error.contains("version"), "unhelpful error: {error}");
    }

    #[tokio::test]
    async fn malformed_json_is_reported_rather_than_half_applied() {
        let db = test_db().await;
        assert!(import_bundle_json(&db.pool, "{ not json").await.is_err());
    }

    #[tokio::test]
    async fn an_empty_bundle_is_valid_and_does_nothing() {
        let db = test_db().await;
        let json = serde_json::json!({
            "version": BUNDLE_VERSION,
            "app": "PastePaw",
            "exported_at": "2026-01-01T00:00:00Z",
            "folders": [],
            "clips": []
        })
        .to_string();

        assert_eq!(
            import_bundle_json(&db.pool, &json).await.expect("import"),
            ImportReport::default()
        );
    }

    #[tokio::test]
    async fn a_clip_title_survives_the_round_trip() {
        let source = test_db().await;
        let folder = make_folder(&source, "work").await;
        insert_clip(&source, "named", "text", "payload", Some(folder)).await;
        sqlx::query("UPDATE clips SET title = ? WHERE uuid = 'named'")
            .bind("Email signature")
            .execute(&source.pool)
            .await
            .expect("set title");

        let json = serde_json::to_string(&export_bundle(&source.pool).await.expect("export"))
            .expect("serialize");

        let target = test_db().await;
        import_bundle_json(&target.pool, &json)
            .await
            .expect("import");

        let title: Option<String> =
            sqlx::query_scalar("SELECT title FROM clips WHERE folder_id IS NOT NULL LIMIT 1")
                .fetch_one(&target.pool)
                .await
                .expect("read title");
        assert_eq!(title.as_deref(), Some("Email signature"));
    }

    /// A bundle written before titles existed has no `title` key at all. It has to keep
    /// importing rather than being rejected, which is what `serde(default)` buys.
    #[tokio::test]
    async fn a_bundle_without_a_title_field_still_imports() {
        let db = test_db().await;
        let json = serde_json::json!({
            "version": BUNDLE_VERSION,
            "app": "PastePaw",
            "exported_at": "2026-01-01T00:00:00Z",
            "folders": [{ "name": "work", "icon": null, "color": null, "is_system": false }],
            "clips": [{
                "clip_type": "text",
                "content": "payload",
                "text_preview": "payload",
                "content_hash": "hash-from-an-older-bundle",
                "folder": "work",
                "source_app": null,
                "metadata": null,
                "created_at": "2026-01-01T00:00:00Z",
                "html": null,
                "rtf_base64": null,
                "image_path": null
            }]
        })
        .to_string();

        let report = import_bundle_json(&db.pool, &json).await.expect("import");

        assert_eq!(report.clips_imported, 1);
    }
}
