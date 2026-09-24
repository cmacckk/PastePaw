use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_x::{start_listening, stop_listening};

use crate::ai::{self, AiAction, AiConfig};
use crate::database::Database;
use crate::models::{Clip, ClipboardItem, Folder, FolderItem};
use crate::settings_manager::SettingsManager;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::Serialize;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

#[tauri::command]
pub async fn ai_process_clip(
    app: AppHandle,
    clip_id: String,
    action: String,
    db: tauri::State<'_, Arc<Database>>,
) -> Result<String, String> {
    let pool = &db.pool;

    // 1. Get Clip
    let clip: Clip = sqlx::query_as(r#"SELECT * FROM clips WHERE uuid = ?"#)
        .bind(&clip_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Clip not found")?;

    let text_content =
        if clip.clip_type == "text" || clip.clip_type == "html" || clip.clip_type == "url" {
            String::from_utf8_lossy(&clip.content).to_string()
        } else {
            return Err("AI processing only supported for text content".to_string());
        };

    // 2. Get AI Config
    let manager = app.state::<Arc<SettingsManager>>();
    let settings = manager.get();

    if settings.ai_api_key.is_empty() {
        return Err("AI API Key is missing in settings".to_string());
    }

    let config = AiConfig {
        provider: settings.ai_provider,
        api_key: settings.ai_api_key,
        model: settings.ai_model,
        base_url: if settings.ai_base_url.is_empty() {
            None
        } else {
            Some(settings.ai_base_url)
        },
    };

    let ai_action = match action.as_str() {
        "summarize" => AiAction::Summarize,
        "translate" => AiAction::Translate,
        "explain_code" => AiAction::ExplainCode,
        "fix_grammar" => AiAction::FixGrammar,
        _ => return Err("Invalid AI action".to_string()),
    };

    let custom_prompt = match ai_action {
        AiAction::Summarize => Some(settings.ai_prompt_summarize),
        AiAction::Translate => Some(settings.ai_prompt_translate),
        AiAction::ExplainCode => Some(settings.ai_prompt_explain_code),
        AiAction::FixGrammar => Some(settings.ai_prompt_fix_grammar),
    };

    // 3. Call AI
    let result = ai::process_text(&text_content, ai_action.clone(), &config, custom_prompt)
        .await
        .map_err(|e| e.to_string())?;

    // 4. Update Metadata
    let mut metadata: serde_json::Value = if let Some(meta_str) = &clip.metadata {
        serde_json::from_str(meta_str).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let key = match ai_action {
        AiAction::Summarize => "ai_summary",
        AiAction::Translate => "ai_translation",
        AiAction::ExplainCode => "ai_explanation",
        AiAction::FixGrammar => "ai_grammar_fix",
    };

    metadata[key] = serde_json::json!(result);
    let new_metadata_str = metadata.to_string();

    sqlx::query("UPDATE clips SET metadata = ? WHERE uuid = ?")
        .bind(&new_metadata_str)
        .bind(&clip_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(result)
}

fn clip_to_list_item(clip: &Clip, image_path: Option<&str>) -> ClipboardItem {
    let content_str = if clip.clip_type == "image" {
        image_path.unwrap_or_default().to_string()
    } else {
        String::from_utf8_lossy(&clip.content).to_string()
    };

    ClipboardItem {
        id: clip.uuid.clone(),
        clip_type: clip.clip_type.clone(),
        content: content_str,
        preview: clip.text_preview.clone(),
        folder_id: clip.folder_id.map(|id| id.to_string()),
        created_at: clip.created_at.to_rfc3339(),
        source_app: clip.source_app.clone(),
        source_icon: clip.source_icon.clone(),
        metadata: clip.metadata.clone(),
    }
}

fn clip_to_detail_item(clip: &Clip, full_image_content: Option<&[u8]>) -> ClipboardItem {
    let content_str = if clip.clip_type == "image" {
        BASE64.encode(full_image_content.unwrap_or(&clip.content))
    } else {
        String::from_utf8_lossy(&clip.content).to_string()
    };

    ClipboardItem {
        id: clip.uuid.clone(),
        clip_type: clip.clip_type.clone(),
        content: content_str,
        preview: clip.text_preview.clone(),
        folder_id: clip.folder_id.map(|id| id.to_string()),
        created_at: clip.created_at.to_rfc3339(),
        source_app: clip.source_app.clone(),
        source_icon: clip.source_icon.clone(),
        metadata: clip.metadata.clone(),
    }
}

async fn delete_clip_image_file_by_uuid(pool: &SqlitePool, clip_uuid: &str) -> Result<(), String> {
    let file_path: Option<String> =
        sqlx::query_scalar(r#"SELECT file_path FROM clip_images WHERE clip_uuid = ?"#)
            .bind(clip_uuid)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;

    if let Some(path) = file_path {
        if !path.is_empty() {
            crate::clipboard::remove_full_image_file(&path);
        }
    }

    Ok(())
}

async fn cleanup_orphan_clip_image_files(pool: &SqlitePool) -> Result<(), String> {
    let orphan_paths: Vec<Option<String>> = sqlx::query_scalar(
        r#"
        SELECT file_path
        FROM clip_images
        WHERE clip_uuid NOT IN (SELECT uuid FROM clips)
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    for path in orphan_paths.into_iter().flatten() {
        if !path.is_empty() {
            crate::clipboard::remove_full_image_file(&path);
        }
    }

    sqlx::query(r#"DELETE FROM clip_images WHERE clip_uuid NOT IN (SELECT uuid FROM clips)"#)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

async fn cleanup_all_clip_image_files(pool: &SqlitePool) -> Result<(), String> {
    let all_paths: Vec<Option<String>> = sqlx::query_scalar(r#"SELECT file_path FROM clip_images"#)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

    for path in all_paths.into_iter().flatten() {
        if !path.is_empty() {
            crate::clipboard::remove_full_image_file(&path);
        }
    }

    Ok(())
}

pub async fn migrate_images_to_files(pool: &SqlitePool) -> Result<(), String> {
    log::info!("Starting background image migration...");

    // 1. Migrate legacy clips (content in 'clips' table)
    let legacy_clips: Vec<Clip> =
        sqlx::query_as(r#"SELECT * FROM clips WHERE clip_type = 'image' AND length(content) > 0"#)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;

    for clip in legacy_clips {
        log::info!("Migrating legacy clip {}...", clip.uuid);
        let full_bytes = clip.content.clone();
        match crate::clipboard::persist_full_image_file(&clip.uuid, &full_bytes) {
            Ok(file_path) => {
                let _ = sqlx::query(
                    r#"
                    INSERT OR REPLACE INTO clip_images (clip_uuid, full_content, file_path, file_size, storage_kind, mime_type, created_at)
                    VALUES (?, x'', ?, ?, 'file', 'image/png', CURRENT_TIMESTAMP)
                    "#,
                )
                .bind(&clip.uuid)
                .bind(&file_path)
                .bind(full_bytes.len() as i64)
                .execute(pool)
                .await;

                let _ = sqlx::query(
                    r#"UPDATE clips SET content = x'', is_thumbnail = 0 WHERE uuid = ?"#,
                )
                .bind(&clip.uuid)
                .execute(pool)
                .await;
            }
            Err(e) => {
                log::error!("Failed to migrate legacy clip {}: {}", clip.uuid, e);
            }
        }
    }

    // 2. Migrate DB-stored images in 'clip_images'
    let db_images: Vec<(String, Vec<u8>)> = sqlx::query_as(
        r#"SELECT clip_uuid, full_content FROM clip_images WHERE storage_kind = 'db' AND length(full_content) > 0"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    for (uuid, content) in db_images {
        log::info!("Migrating DB-stored image for clip {}...", uuid);
        match crate::clipboard::persist_full_image_file(&uuid, &content) {
            Ok(file_path) => {
                let _ = sqlx::query(
                    r#"
                    UPDATE clip_images
                    SET full_content = x'', file_path = ?, storage_kind = 'file'
                    WHERE clip_uuid = ?
                    "#,
                )
                .bind(&file_path)
                .bind(&uuid)
                .execute(pool)
                .await;
            }
            Err(e) => {
                log::error!("Failed to migrate DB image for clip {}: {}", uuid, e);
            }
        }
    }

    log::info!("Background image migration completed.");
    Ok(())
}

async fn load_full_image_content(pool: &SqlitePool, clip: &mut Clip) -> Result<Vec<u8>, String> {
    if clip.clip_type != "image" {
        return Err("Clip is not an image".to_string());
    }

    // 1. Try fetching from file path in DB
    let file_path: Option<String> =
        sqlx::query_scalar(r#"SELECT file_path FROM clip_images WHERE clip_uuid = ?"#)
            .bind(&clip.uuid)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;

    if let Some(path) = file_path {
        if !path.is_empty() {
            // If file exists, return it
            if let Ok(bytes) = crate::clipboard::read_full_image_file(&path) {
                return Ok(bytes);
            }
            // If file missing, try fallbacks below
            log::warn!("Image file missing at {}, checking DB backups...", path);
        }
    }

    // 2. Try DB blob (migration not done or failed)
    let full_content: Option<Vec<u8>> =
        sqlx::query_scalar(r#"SELECT full_content FROM clip_images WHERE clip_uuid = ?"#)
            .bind(&clip.uuid)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;

    if let Some(content) = full_content {
        if !content.is_empty() {
            return Ok(content);
        }
    }

    // 3. Legacy content in clips table
    if !clip.content.is_empty() {
        return Ok(clip.content.clone());
    }

    Err("Image content missing".to_string())
}

/// The clip types a filter may select.
///
/// Mirrors what capture actually produces. There is no link or colour type: those
/// would need their own capture paths, not a filter value.
pub const CLIP_TYPES: [&str; 3] = ["text", "image", "file"];

/// Keeps recognised type names and reports the rest.
///
/// An unrecognised name is dropped rather than reaching SQL, so a bad value from the
/// window narrows nothing rather than silently matching nothing.
fn normalise_clip_types(requested: Option<Vec<String>>) -> Vec<String> {
    let requested = requested.unwrap_or_default();
    let mut kept = Vec::with_capacity(requested.len());

    for name in requested {
        if CLIP_TYPES.contains(&name.as_str()) {
            kept.push(name);
        } else {
            log::warn!("Ignoring an unknown clip type filter: {name}");
        }
    }

    kept
}

/// Appends `AND <column> IN (?, ?, ...)` for a non-empty list.
///
/// `column` is always a literal from this file and never comes from the caller, which
/// is what makes pushing it raw safe; the values themselves are always bound.
fn push_in_list(builder: &mut QueryBuilder<'_, Sqlite>, column: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }

    builder.push(" AND ").push(column).push(" IN (");
    let mut separated = builder.separated(", ");
    for value in values {
        separated.push_bind(value.clone());
    }
    separated.push_unseparated(")");
}

/// Appends the folder, type and source application conditions shared by the list and
/// search queries.
fn push_clip_filters(
    builder: &mut QueryBuilder<'_, Sqlite>,
    folder_id: Option<i64>,
    clip_types: &[String],
    source_apps: &[String],
) {
    if let Some(folder_id) = folder_id {
        builder.push(" AND folder_id = ").push_bind(folder_id);
    }

    push_in_list(builder, "clip_type", clip_types);
    push_in_list(builder, "source_app", source_apps);
}

/// Resolves the folder filter, separating "no filter" from "unparseable id".
fn resolve_folder_filter(filter_id: Option<&str>) -> Result<Option<i64>, String> {
    match filter_id {
        None => Ok(None),
        Some(id) => id
            .parse::<i64>()
            .map(Some)
            .map_err(|_| format!("Unknown folder id: {id}")),
    }
}

/// Lists clips for the grid, narrowed to a folder and/or a set of clip types.
///
/// Built with `QueryBuilder` rather than by concatenating SQL, because the type filter
/// is a variable length list. Kept out of the command so the filter behaviour can be
/// tested against a real database instead of only through the window.
pub async fn query_clips(
    pool: &SqlitePool,
    folder_id: Option<i64>,
    clip_types: &[String],
    source_apps: &[String],
    limit: i64,
    offset: i64,
) -> Result<Vec<Clip>, String> {
    let mut builder: QueryBuilder<'_, Sqlite> =
        QueryBuilder::new("SELECT * FROM clips WHERE is_deleted = 0");
    push_clip_filters(&mut builder, folder_id, clip_types, source_apps);
    builder
        .push(" ORDER BY created_at DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);

    builder
        .build_query_as::<Clip>()
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())
}

/// The search counterpart of [`query_clips`].
pub async fn query_clips_matching(
    pool: &SqlitePool,
    pattern: &str,
    folder_id: Option<i64>,
    clip_types: &[String],
    source_apps: &[String],
    limit: i64,
    offset: i64,
) -> Result<Vec<Clip>, String> {
    let mut builder: QueryBuilder<'_, Sqlite> =
        QueryBuilder::new("SELECT * FROM clips WHERE is_deleted = 0 AND (text_preview LIKE ");
    builder
        .push_bind(pattern.to_string())
        .push(" OR content LIKE ")
        .push_bind(pattern.to_string())
        .push(")");
    push_clip_filters(&mut builder, folder_id, clip_types, source_apps);
    builder
        .push(" ORDER BY created_at DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);

    builder
        .build_query_as::<Clip>()
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())
}

/// Normalises the source application filter.
///
/// These are arbitrary application names rather than a fixed set, so there is nothing
/// to validate against; empty values are dropped and the list is capped so a
/// pathological request cannot build an enormous statement.
fn normalise_source_apps(requested: Option<Vec<String>>) -> Vec<String> {
    const MAX_SOURCE_APPS: usize = 64;

    requested
        .unwrap_or_default()
        .into_iter()
        .filter(|name| !name.trim().is_empty())
        .take(MAX_SOURCE_APPS)
        .collect()
}

/// A source application that has clips, with how many.
#[derive(Debug, Serialize)]
pub struct SourceAppCount {
    pub name: String,
    pub count: i64,
}

/// Lists the source applications that actually have clips, so the filter can offer
/// real choices instead of a free text box.
async fn query_source_apps(pool: &SqlitePool) -> Result<Vec<SourceAppCount>, String> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"SELECT source_app, COUNT(*) AS clips
           FROM clips
           WHERE is_deleted = 0 AND source_app IS NOT NULL AND source_app != ''
           GROUP BY source_app
           ORDER BY clips DESC, source_app"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .into_iter()
        .map(|(name, count)| SourceAppCount { name, count })
        .collect())
}

/// Lists the source applications offered by the filter.
#[tauri::command]
pub async fn get_source_apps(
    db: tauri::State<'_, Arc<Database>>,
) -> Result<Vec<SourceAppCount>, String> {
    query_source_apps(&db.pool).await
}

#[tauri::command]
pub async fn get_clips(
    filter_id: Option<String>,
    limit: i64,
    offset: i64,
    preview_only: Option<bool>,
    filter_types: Option<Vec<String>>,
    filter_source_apps: Option<Vec<String>>,
    db: tauri::State<'_, Arc<Database>>,
) -> Result<Vec<ClipboardItem>, String> {
    let pool = &db.pool;
    let preview_only = preview_only.unwrap_or(false);
    let started = Instant::now();

    log::info!(
        "get_clips called with filter_id: {:?}, preview_only: {}",
        filter_id,
        preview_only
    );

    let folder_id = match resolve_folder_filter(filter_id.as_deref()) {
        Ok(folder_id) => folder_id,
        Err(message) => {
            // Kept from before: an unparseable folder id yields an empty page rather than
            // every clip. Logged now, because silence made it look like an empty folder.
            log::warn!("{message}");
            return Ok(Vec::new());
        }
    };

    let clip_types = normalise_clip_types(filter_types);
    let source_apps = normalise_source_apps(filter_source_apps);
    log::info!(
        "Querying clips: folder={folder_id:?} types={clip_types:?} apps={source_apps:?} offset={offset} limit={limit}"
    );

    let sql_started = Instant::now();
    let clips = query_clips(pool, folder_id, &clip_types, &source_apps, limit, offset).await?;
    let sql_ms = sql_started.elapsed().as_millis();

    log::info!("DB: Found {} clips", clips.len());

    // Batch fetch image paths
    let mut image_path_map: HashMap<String, String> = HashMap::new();
    let image_uuids: Vec<String> = clips
        .iter()
        .filter(|c| c.clip_type == "image")
        .map(|c| c.uuid.clone())
        .collect();

    if !image_uuids.is_empty() {
        // Construct query: SELECT clip_uuid, file_path FROM clip_images WHERE clip_uuid IN (?, ?, ...)
        let placeholders: Vec<String> = image_uuids.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            "SELECT clip_uuid, file_path FROM clip_images WHERE clip_uuid IN ({})",
            placeholders.join(",")
        );

        let mut query_builder = sqlx::query_as::<_, (String, Option<String>)>(&query);
        for uuid in &image_uuids {
            query_builder = query_builder.bind(uuid);
        }

        let results = query_builder
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
        for (uuid, path) in results {
            if let Some(p) = path {
                if !p.is_empty() {
                    image_path_map.insert(uuid, p);
                }
            }
        }
    }

    let image_rows = image_uuids.len();
    let raw_bytes: usize = clips.iter().map(|clip| clip.content.len()).sum();
    let map_started = Instant::now();
    let items: Vec<ClipboardItem> = clips
        .iter()
        .enumerate()
        .map(|(idx, clip)| {
            let item = clip_to_list_item(clip, image_path_map.get(&clip.uuid).map(|s| s.as_str()));
            // Only log first 10 clips to reduce noise
            if idx < 10 {
                log::trace!(
                    "{} Clip {}: type='{}', content_len={}",
                    idx,
                    clip.uuid,
                    clip.clip_type,
                    item.content.len()
                );
            }
            item
        })
        .collect();
    let map_ms = map_started.elapsed().as_millis();
    let total_ms = started.elapsed().as_millis();
    log::info!(
        "[perf][get_clips] sql_ms={} map_ms={} total_ms={} rows={} images={} raw_bytes={} preview_only={} filter_id={:?} offset={} limit={}",
        sql_ms,
        map_ms,
        total_ms,
        clips.len(),
        image_rows,
        raw_bytes,
        preview_only,
        filter_id,
        offset,
        limit
    );

    Ok(items)
}

#[tauri::command]
pub async fn get_clip(
    id: String,
    db: tauri::State<'_, Arc<Database>>,
) -> Result<ClipboardItem, String> {
    let pool = &db.pool;

    let clip: Option<Clip> = sqlx::query_as(r#"SELECT * FROM clips WHERE uuid = ?"#)
        .bind(&id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    match clip {
        Some(mut clip) => {
            if clip.clip_type == "image" {
                let full = load_full_image_content(pool, &mut clip).await?;
                Ok(clip_to_detail_item(&clip, Some(&full)))
            } else {
                Ok(clip_to_detail_item(&clip, None))
            }
        }
        None => Err("Clip not found".to_string()),
    }
}

// TODO(xueshi) get_clip is same as get_clip_detail???
#[tauri::command]
pub async fn get_clip_detail(
    id: String,
    db: tauri::State<'_, Arc<Database>>,
) -> Result<ClipboardItem, String> {
    get_clip(id, db).await
}

/// Reads every stored alternate format for a clip as `(format, content)` pairs.
///
/// `clips.content` only holds the plain text rendition that search and the preview
/// read; the richer forms paste-back needs live in `clip_formats`.
async fn load_clip_formats(pool: &SqlitePool, clip_uuid: &str) -> Vec<(String, Vec<u8>)> {
    match sqlx::query_as::<_, (String, Vec<u8>)>(
        r#"SELECT format, content FROM clip_formats WHERE clip_uuid = ?"#,
    )
    .bind(clip_uuid)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            log::error!("Failed to load stored formats for clip {clip_uuid}: {e}");
            Vec::new()
        }
    }
}

#[tauri::command]
pub async fn paste_clip(
    id: String,
    app: AppHandle,
    window: tauri::WebviewWindow,
    db: tauri::State<'_, Arc<Database>>,
) -> Result<(), String> {
    let pool = &db.pool;

    let clip: Option<Clip> = sqlx::query_as(r#"SELECT * FROM clips WHERE uuid = ?"#)
        .bind(&id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    match clip {
        Some(mut clip) => {
            // Synchronize clipboard access across the app
            let _guard = crate::clipboard::CLIPBOARD_SYNC.lock().await;

            let content_hash = clip.content_hash.clone();
            let uuid = clip.uuid.clone();

            // Stop monitor
            if let Err(e) = stop_listening().await {
                log::error!("Failed to stop listener: {}", e);
            }

            let mut final_res = Ok(());

            crate::clipboard::set_ignore_hash(content_hash.clone());

            // Every clip type now goes through the same writer, so the clipboard is
            // replaced exactly once with the full set of formats this clip has.
            let mut payload = crate::clipboard::ClipboardPayload::default();
            match clip.clip_type.as_str() {
                "image" => match load_full_image_content(pool, &mut clip).await {
                    Ok(png) => payload.image_png = Some(png),
                    Err(e) => final_res = Err(e),
                },
                kind => {
                    // Everything that is not an image can carry stored alternates.
                    for (format, content) in load_clip_formats(pool, &uuid).await {
                        match format.as_str() {
                            crate::clipboard_formats::FORMAT_FILE => {
                                match serde_json::from_slice::<Vec<String>>(&content) {
                                    Ok(files) if !files.is_empty() => payload.files = Some(files),
                                    _ => log::error!(
                                        "Stored file list for clip {uuid} is empty or invalid"
                                    ),
                                }
                            }
                            crate::clipboard_formats::FORMAT_HTML => {
                                payload.html = Some(String::from_utf8_lossy(&content).into_owned())
                            }
                            crate::clipboard_formats::FORMAT_RTF => payload.rtf = Some(content),
                            other => log::debug!("Ignoring unknown stored format {other}"),
                        }
                    }

                    if kind == "file" {
                        // The stored list is the only way to reproduce a file clip, so
                        // refuse rather than silently copying nothing.
                        if payload.files.is_none() {
                            final_res = Err("This file clip has no stored file list".to_string());
                        }
                    } else {
                        payload.text = Some(String::from_utf8_lossy(&clip.content).to_string());
                    }
                }
            }

            if final_res.is_ok() {
                let mut last_err = String::new();
                for i in 0..5 {
                    match crate::clipboard::write_clipboard_payload(&payload) {
                        Ok(()) => {
                            last_err.clear();
                            break;
                        }
                        Err(e) => {
                            last_err = e;
                            log::warn!(
                                "Clipboard write attempt {} failed: {}. Retrying...",
                                i + 1,
                                last_err
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        }
                    }
                }
                if !last_err.is_empty() {
                    final_res = Err(format!("Failed to write clip to clipboard: {last_err}"));
                }
            }

            // Manually perform the LRU bump (update created_at)
            let _ =
                sqlx::query(r#"UPDATE clips SET created_at = CURRENT_TIMESTAMP WHERE uuid = ?"#)
                    .bind(&uuid)
                    .execute(pool)
                    .await;

            // Restart monitor
            let app_clone = app.clone();
            if let Err(e) = start_listening(app_clone).await {
                log::error!("Failed to restart listener: {}", e);
            }

            if final_res.is_ok() {
                let content = if clip.clip_type == "image" {
                    "[Image]".to_string()
                } else {
                    String::from_utf8_lossy(&clip.content).to_string()
                };
                let _ = window.emit("clipboard-write", &content);

                // Check auto_paste setting
                let manager = app.state::<Arc<SettingsManager>>();
                let settings = manager.get();
                let auto_paste = settings.auto_paste;
                let paste_method = settings.paste_method.clone();
                log::info!(
                    "paste_clip: auto_paste={}, paste_method={}",
                    auto_paste,
                    paste_method
                );

                if auto_paste {
                    // Auto-Paste Logic
                    // 1. Hide window immediately to trigger focus switch to previous app
                    crate::animate_window_hide(
                        &window,
                        Some(Box::new(move || {
                            // 2. Callback executed AFTER window is hidden
                            // Small buffer to ensure OS focus switch is complete
                            std::thread::sleep(std::time::Duration::from_millis(200));
                            crate::clipboard::send_paste_input(&paste_method);
                        })),
                    );
                } else {
                    crate::animate_window_hide(&window, None);
                }
            }
            final_res
        }
        None => Err("Clip not found".to_string()),
    }
}

#[tauri::command]
pub async fn delete_clip(
    id: String,
    hard_delete: bool,
    db: tauri::State<'_, Arc<Database>>,
) -> Result<(), String> {
    let pool = &db.pool;

    if hard_delete {
        delete_clip_image_file_by_uuid(pool, &id).await?;

        sqlx::query(r#"DELETE FROM clip_images WHERE clip_uuid = ?"#)
            .bind(&id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query(r#"DELETE FROM clips WHERE uuid = ?"#)
            .bind(&id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        sqlx::query(r#"UPDATE clips SET is_deleted = 1 WHERE uuid = ?"#)
            .bind(&id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Saves a clip into the built-in folder, or takes it back out.
///
/// Returns the state after the toggle so the window does not have to guess at it.
#[tauri::command]
pub async fn toggle_pin(
    clip_id: String,
    db: tauri::State<'_, Arc<Database>>,
    window: tauri::WebviewWindow,
) -> Result<bool, String> {
    let is_pinned = crate::pins::toggle_pin(&db.pool, &clip_id).await?;

    // The grid groups by folder and the folder counts include this clip, so both
    // the list and the sidebar need to reload.
    let _ = window.emit("clipboard-change", ());

    log::info!("toggle_pin: clip {clip_id} pinned={is_pinned}");

    Ok(is_pinned)
}

#[tauri::command]
pub async fn move_to_folder(
    clip_id: String,
    folder_id: Option<String>,
    db: tauri::State<'_, Arc<Database>>,
) -> Result<(), String> {
    let pool = &db.pool;

    let folder_id = match folder_id {
        Some(id) => Some(id.parse::<i64>().map_err(|_| "Invalid folder ID")?),
        None => None,
    };

    sqlx::query(r#"UPDATE clips SET folder_id = ? WHERE uuid = ?"#)
        .bind(folder_id)
        .bind(&clip_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn create_folder(
    name: String,
    icon: Option<String>,
    color: Option<String>,
    db: tauri::State<'_, Arc<Database>>,
    window: tauri::WebviewWindow,
) -> Result<FolderItem, String> {
    let pool = &db.pool;

    // Check if folder with same name exists (excluding system folders if we wanted, but name uniqueness is good generally)
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM folders WHERE name = ?")
        .bind(&name)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    if exists.is_some() {
        return Err("A folder with this name already exists".to_string());
    }

    let id = sqlx::query(r#"INSERT INTO folders (name, icon, color) VALUES (?, ?, ?)"#)
        .bind(&name)
        .bind(icon.as_ref())
        .bind(color.as_ref())
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?
        .last_insert_rowid();

    let _ = window.app_handle().emit("clipboard-change", ());

    Ok(FolderItem {
        id: id.to_string(),
        name,
        icon,
        color,
        is_system: false,
        item_count: 0,
    })
}

#[tauri::command]
pub async fn delete_folder(
    id: String,
    db: tauri::State<'_, Arc<Database>>,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    let pool = &db.pool;

    let folder_id: i64 = id.parse().map_err(|_| "Invalid folder ID")?;

    // The built-in folder holds every pinned clip, so deleting it would take those
    // clips with it. It is not the user's to remove.
    let is_system: Option<i64> = sqlx::query_scalar("SELECT is_system FROM folders WHERE id = ?")
        .bind(folder_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;

    match is_system {
        None => return Err("Folder not found".to_string()),
        Some(flag) if flag != 0 => return Err("The built-in folder cannot be deleted".to_string()),
        Some(_) => {}
    }

    // Fetch image file paths BEFORE deleting clips, because ON DELETE CASCADE
    // on clip_images will remove the rows automatically, leaking files on disk.
    let paths_to_delete: Vec<Option<String>> = sqlx::query_scalar(
        r#"SELECT ci.file_path FROM clip_images ci
           JOIN clips c ON ci.clip_uuid = c.uuid
           WHERE c.folder_id = ?"#,
    )
    .bind(folder_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    for path in paths_to_delete.into_iter().flatten() {
        if !path.is_empty() {
            crate::clipboard::remove_full_image_file(&path);
        }
    }

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query(r#"DELETE FROM clips WHERE folder_id = ?"#)
        .bind(folder_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query(r#"DELETE FROM folders WHERE id = ?"#)
        .bind(folder_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    tx.commit().await.map_err(|e| e.to_string())?;

    let _ = window.app_handle().emit("clipboard-change", ());
    Ok(())
}

#[tauri::command]
pub async fn rename_folder(
    id: String,
    name: String,
    db: tauri::State<'_, Arc<Database>>,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    let pool = &db.pool;

    let folder_id: i64 = id.parse().map_err(|_| "Invalid folder ID")?;

    // Check availability
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM folders WHERE name = ? AND id != ?")
            .bind(&name)
            .bind(folder_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;

    if exists.is_some() {
        return Err("A folder with this name already exists".to_string());
    }

    sqlx::query(r#"UPDATE folders SET name = ? WHERE id = ?"#)
        .bind(name)
        .bind(folder_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    // Emit event so main window knows to refresh
    let _ = window.app_handle().emit("clipboard-change", ());
    Ok(())
}

#[tauri::command]
pub async fn search_clips(
    query: String,
    filter_id: Option<String>,
    filter_types: Option<Vec<String>>,
    filter_source_apps: Option<Vec<String>>,
    limit: i64,
    offset: i64,
    db: tauri::State<'_, Arc<Database>>,
) -> Result<Vec<ClipboardItem>, String> {
    let pool = &db.pool;
    let started = Instant::now();

    let search_pattern = format!("%{}%", query);

    let folder_id = match resolve_folder_filter(filter_id.as_deref()) {
        Ok(folder_id) => folder_id,
        Err(message) => {
            log::warn!("{message}");
            return Ok(Vec::new());
        }
    };

    let sql_started = Instant::now();
    let clips = query_clips_matching(
        pool,
        &search_pattern,
        folder_id,
        &normalise_clip_types(filter_types),
        &normalise_source_apps(filter_source_apps),
        limit,
        offset,
    )
    .await?;
    let sql_ms = sql_started.elapsed().as_millis();

    // Batch fetch image paths
    let mut image_path_map: HashMap<String, String> = HashMap::new();
    let image_uuids: Vec<String> = clips
        .iter()
        .filter(|c| c.clip_type == "image")
        .map(|c| c.uuid.clone())
        .collect();

    if !image_uuids.is_empty() {
        let placeholders: Vec<String> = image_uuids.iter().map(|_| "?".to_string()).collect();
        let query = format!(
            "SELECT clip_uuid, file_path FROM clip_images WHERE clip_uuid IN ({})",
            placeholders.join(",")
        );

        let mut query_builder = sqlx::query_as::<_, (String, Option<String>)>(&query);
        for uuid in &image_uuids {
            query_builder = query_builder.bind(uuid);
        }

        let results = query_builder
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;
        for (uuid, path) in results {
            if let Some(p) = path {
                if !p.is_empty() {
                    image_path_map.insert(uuid, p);
                }
            }
        }
    }

    let image_rows = image_uuids.len();
    let raw_bytes: usize = clips.iter().map(|clip| clip.content.len()).sum();
    let map_started = Instant::now();
    let items: Vec<ClipboardItem> = clips
        .iter()
        .map(|clip| clip_to_list_item(clip, image_path_map.get(&clip.uuid).map(|s| s.as_str())))
        .collect();
    let map_ms = map_started.elapsed().as_millis();
    let total_ms = started.elapsed().as_millis();
    log::info!(
        "[perf][search_clips] sql_ms={} map_ms={} total_ms={} rows={} images={} raw_bytes={} filter_id={:?} offset={} limit={}",
        sql_ms,
        map_ms,
        total_ms,
        clips.len(),
        image_rows,
        raw_bytes,
        filter_id,
        offset,
        limit
    );

    Ok(items)
}

#[tauri::command]
pub async fn get_folders(db: tauri::State<'_, Arc<Database>>) -> Result<Vec<FolderItem>, String> {
    let pool = &db.pool;

    let folders: Vec<Folder> = sqlx::query_as(r#"SELECT * FROM folders ORDER BY created_at"#)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

    // Get counts for all folders in one query
    let counts: Vec<(i64, i64)> = sqlx::query_as(
        r#"
        SELECT folder_id, COUNT(*) as count
        FROM clips
        WHERE is_deleted = 0 AND folder_id IS NOT NULL
        GROUP BY folder_id
    "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Create a map for easier lookup
    let count_map: HashMap<i64, i64> = counts.into_iter().collect();

    let items: Vec<FolderItem> = folders
        .iter()
        .map(|folder| FolderItem {
            id: folder.id.to_string(),
            name: folder.name.clone(),
            icon: folder.icon.clone(),
            color: folder.color.clone(),
            is_system: folder.is_system,
            item_count: *count_map.get(&folder.id).unwrap_or(&0),
        })
        .collect();

    //println!("folder items: {:#?}", items);

    Ok(items)
}

#[tauri::command]
pub fn hide_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ping() -> Result<String, String> {
    Ok("pong".to_string())
}

#[tauri::command]
pub fn test_log() -> Result<String, String> {
    log::trace!("[TEST] Trace level log");
    log::debug!("[TEST] Debug level log");
    log::info!("[TEST] Info level log");
    log::warn!("[TEST] Warn level log");
    log::error!("[TEST] Error level log");
    Ok("Logs emitted - check console".to_string())
}

#[tauri::command]
pub async fn get_clipboard_history_size(
    db: tauri::State<'_, Arc<Database>>,
) -> Result<i64, String> {
    let pool = &db.pool;

    let count: i64 =
        sqlx::query_scalar::<_, i64>(r#"SELECT COUNT(*) FROM clips WHERE is_deleted = 0"#)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    Ok(count)
}

/// Applies the retention policy right away and reports how many clips it removed.
///
/// The policy already runs at startup and on every ingest; this exists so the
/// settings window can offer a "clean up now" action.
#[tauri::command]
pub async fn prune_history(
    db: tauri::State<'_, Arc<Database>>,
    window: tauri::WebviewWindow,
) -> Result<i64, String> {
    let settings = window.state::<Arc<SettingsManager>>().get();

    let report =
        crate::retention::prune(&db.pool, settings.auto_delete_days, settings.max_items).await?;

    if report.total() > 0 {
        // Whatever the window is showing still includes the pruned clips.
        let _ = window.emit("clipboard-change", ());
    }

    log::info!(
        "prune_history: removed {} clip(s) ({} expired, {} over the item limit)",
        report.total(),
        report.by_age,
        report.by_count
    );

    Ok(report.total() as i64)
}

/// Writes every folder clip to a JSON file the user picks, and returns its path.
///
/// Deliberately a synchronous command. It opens a modal file dialog, and blocking a
/// worker of the async runtime for as long as that dialog is open would stall the
/// clipboard monitor along with everything else sharing that runtime.
#[tauri::command]
pub fn export_folders(
    app: AppHandle,
    db: tauri::State<'_, Arc<Database>>,
) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;

    let bundle = crate::get_runtime()
        .expect("the global runtime is created during startup")
        .block_on(crate::export::export_bundle(&db.pool))?;

    let clip_count = bundle.clips.len();
    let json = serde_json::to_string_pretty(&bundle).map_err(|e| e.to_string())?;

    let path = app
        .dialog()
        .file()
        .add_filter("JSON", &["json"])
        .set_file_name(crate::export::suggested_file_name())
        .blocking_save_file()
        .ok_or_else(|| "No file selected".to_string())?
        .into_path()
        .map_err(|e| e.to_string())?;

    std::fs::write(&path, json).map_err(|e| format!("could not write {}: {e}", path.display()))?;

    log::info!(
        "export_folders: wrote {clip_count} clip(s) to {}",
        path.display()
    );

    Ok(path.to_string_lossy().into_owned())
}

/// Applies a bundle the user picks, and reports what it did.
#[tauri::command]
pub fn import_folders(
    app: AppHandle,
    db: tauri::State<'_, Arc<Database>>,
) -> Result<crate::export::ImportReport, String> {
    use tauri_plugin_dialog::DialogExt;

    let path = app
        .dialog()
        .file()
        .add_filter("JSON", &["json"])
        .blocking_pick_file()
        .ok_or_else(|| "No file selected".to_string())?
        .into_path()
        .map_err(|e| e.to_string())?;

    let json = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;

    let report = crate::get_runtime()
        .expect("the global runtime is created during startup")
        .block_on(crate::export::import_bundle_json(&db.pool, &json))?;

    // The main window keeps its own copy of the list, so it has to be told that
    // folders and clips appeared underneath it.
    if report.clips_imported > 0 || report.folders_created > 0 {
        let _ = app.emit("clipboard-change", ());
    }

    log::info!(
        "import_folders: imported {} clip(s), skipped {}, folders created {} / reused {}, images restored {} / missing {}",
        report.clips_imported,
        report.clips_skipped,
        report.folders_created,
        report.folders_reused,
        report.images_restored,
        report.images_missing
    );

    Ok(report)
}

#[tauri::command]
pub async fn clear_clipboard_history(db: tauri::State<'_, Arc<Database>>) -> Result<(), String> {
    let pool = &db.pool;

    sqlx::query(r#"DELETE FROM clips WHERE is_deleted = 1"#)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    cleanup_orphan_clip_image_files(pool).await?;
    Ok(())
}

#[tauri::command]
pub async fn clear_all_clips(
    preserve_folders: Option<bool>,
    db: tauri::State<'_, Arc<Database>>,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    let pool = &db.pool;
    let should_preserve = preserve_folders.unwrap_or(true);

    if should_preserve {
        // Fetch image file paths BEFORE deleting clips, because ON DELETE CASCADE
        // on clip_images will remove the rows automatically, leaking files on disk.
        let paths_to_delete: Vec<Option<String>> = sqlx::query_scalar(
            r#"SELECT ci.file_path FROM clip_images ci
               JOIN clips c ON ci.clip_uuid = c.uuid
               WHERE c.folder_id IS NULL"#,
        )
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

        for path in paths_to_delete.into_iter().flatten() {
            if !path.is_empty() {
                crate::clipboard::remove_full_image_file(&path);
            }
        }

        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

        sqlx::query(r#"DELETE FROM clips WHERE folder_id IS NULL"#)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        tx.commit().await.map_err(|e| e.to_string())?;
    } else {
        cleanup_all_clip_image_files(pool).await?;

        let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

        sqlx::query(r#"DELETE FROM clip_images"#)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query(r#"DELETE FROM clips"#)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;

        tx.commit().await.map_err(|e| e.to_string())?;
    }

    let _ = window.app_handle().emit("clipboard-change", ());
    Ok(())
}

#[tauri::command]
pub async fn remove_duplicate_clips(db: tauri::State<'_, Arc<Database>>) -> Result<i64, String> {
    let pool = &db.pool;

    let result = sqlx::query(
        r#"
        DELETE FROM clips
        WHERE id NOT IN (
            SELECT MIN(id)
            FROM clips
            GROUP BY content_hash
        )
    "#,
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    cleanup_orphan_clip_image_files(pool).await?;

    Ok(result.rows_affected() as i64)
}

#[tauri::command]
pub async fn register_global_shortcut(
    hotkey: String,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    use tauri_plugin_global_shortcut::ShortcutState;

    let app = window.app_handle();
    let shortcut = Shortcut::from_str(&hotkey).map_err(|e| format!("Invalid hotkey: {:?}", e))?;

    if let Err(e) = app.global_shortcut().unregister_all() {
        log::warn!("Failed to unregister existing shortcuts: {:?}", e);
    }

    let main_window = app
        .get_webview_window("main")
        .ok_or_else(|| "Main window not found".to_string())?;

    let win_clone = main_window.clone();
    if let Err(e) = app
        .global_shortcut()
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                if win_clone.is_visible().unwrap_or(false)
                    && win_clone.is_focused().unwrap_or(false)
                {
                    crate::animate_window_hide(&win_clone, None);
                } else {
                    crate::position_window_at_bottom(&win_clone);
                }
            }
        })
    {
        return Err(format!("Failed to register hotkey: {:?}", e));
    }

    log::info!("Registered global shortcut: {}", hotkey);
    Ok(())
}

#[tauri::command]
pub async fn refresh_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        let win_for_show = win.clone();
        crate::animate_window_hide(
            &win,
            Some(Box::new(move || {
                crate::position_window_at_bottom(&win_for_show);
            })),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn focus_window(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(&label) {
        if let Err(e) = window.unminimize() {
            log::warn!("Failed to unminimize window {}: {:?}", label, e);
        }
        if let Err(e) = window.show() {
            log::warn!("Failed to show window {}: {:?}", label, e);
        }
        if let Err(e) = window.set_focus() {
            log::warn!("Failed to focus window {}: {:?}", label, e);
        }

        Ok(())
    } else {
        Err(format!("Window {} not found", label))
    }
}

#[tauri::command]
pub fn show_window(window: tauri::WebviewWindow) -> Result<(), String> {
    crate::position_window_at_bottom(&window);
    Ok(())
}

#[tauri::command]
pub async fn pick_file(app: AppHandle) -> Result<String, String> {
    use tauri_plugin_dialog::DialogExt;

    let file_path = app
        .dialog()
        .file()
        .add_filter("Executables", &["exe", "app"])
        .blocking_pick_file();

    match file_path {
        Some(path) => Ok(path.to_string()),
        None => Err("No file selected".to_string()),
    }
}

#[tauri::command]
pub fn get_layout_config() -> serde_json::Value {
    serde_json::json!({
        "window_height": crate::constants::WINDOW_HEIGHT,
    })
}

#[tauri::command]
pub async fn get_available_update(
    app: AppHandle,
    update_manager: tauri::State<'_, Arc<crate::updater::UpdateManager>>,
) -> Result<Option<crate::updater::UpdateInfo>, String> {
    Ok(update_manager.get_available_version(&app).await)
}

#[tauri::command]
pub async fn check_update_now(
    app: AppHandle,
    update_manager: tauri::State<'_, Arc<crate::updater::UpdateManager>>,
) -> Result<Option<crate::updater::UpdateInfo>, String> {
    update_manager.check_for_updates(&app).await
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    update_manager: tauri::State<'_, Arc<crate::updater::UpdateManager>>,
) -> Result<(), String> {
    update_manager.install_update(&app).await
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

    /// `age` is a SQLite datetime modifier such as `-2 days`.
    async fn insert_clip(
        db: &Database,
        uuid: &str,
        clip_type: &str,
        folder: Option<i64>,
        age: &str,
    ) {
        let body = format!("body of {uuid}");
        sqlx::query(
            r#"INSERT INTO clips (uuid, clip_type, content, content_hash, text_preview, folder_id,
                                  created_at, last_accessed)
               VALUES (?, ?, ?, ?, ?, ?, datetime('now', ?), CURRENT_TIMESTAMP)"#,
        )
        .bind(uuid)
        .bind(clip_type)
        .bind(body.as_bytes())
        .bind(format!("hash-{uuid}"))
        .bind(&body)
        .bind(folder)
        .bind(age)
        .execute(&db.pool)
        .await
        .expect("insert clip");
    }

    async fn make_folder(db: &Database, name: &str) -> i64 {
        sqlx::query_scalar("INSERT INTO folders (name) VALUES (?) RETURNING id")
            .bind(name)
            .fetch_one(&db.pool)
            .await
            .expect("insert folder")
    }

    fn uuids(clips: &[Clip]) -> Vec<String> {
        clips.iter().map(|clip| clip.uuid.clone()).collect()
    }

    #[tokio::test]
    async fn no_type_filter_returns_every_type() {
        let db = test_db().await;
        insert_clip(&db, "a-text", "text", None, "-1 days").await;
        insert_clip(&db, "b-image", "image", None, "-2 days").await;
        insert_clip(&db, "c-file", "file", None, "-3 days").await;

        let clips = query_clips(&db.pool, None, &[], &[], 20, 0)
            .await
            .expect("query");

        assert_eq!(clips.len(), 3);
    }

    #[tokio::test]
    async fn the_type_filter_narrows_to_the_requested_types() {
        let db = test_db().await;
        insert_clip(&db, "a-text", "text", None, "-1 days").await;
        insert_clip(&db, "b-image", "image", None, "-2 days").await;
        insert_clip(&db, "c-file", "file", None, "-3 days").await;

        let clips = query_clips(
            &db.pool,
            None,
            &["image".to_string(), "file".to_string()],
            &[],
            20,
            0,
        )
        .await
        .expect("query");

        assert_eq!(uuids(&clips), vec!["b-image", "c-file"]);
    }

    #[tokio::test]
    async fn the_type_filter_combines_with_the_folder_filter() {
        let db = test_db().await;
        let pinned = make_folder(&db, "pinned").await;
        insert_clip(&db, "filed-image", "image", Some(pinned), "-1 days").await;
        insert_clip(&db, "filed-text", "text", Some(pinned), "-2 days").await;
        insert_clip(&db, "loose-image", "image", None, "-3 days").await;

        let clips = query_clips(&db.pool, Some(pinned), &["image".to_string()], &[], 20, 0)
            .await
            .expect("query");

        assert_eq!(uuids(&clips), vec!["filed-image"]);
    }

    #[tokio::test]
    async fn results_are_newest_first_and_paged() {
        let db = test_db().await;
        insert_clip(&db, "newest", "text", None, "-1 days").await;
        insert_clip(&db, "middle", "text", None, "-2 days").await;
        insert_clip(&db, "oldest", "text", None, "-3 days").await;

        let first_page = query_clips(&db.pool, None, &[], &[], 2, 0)
            .await
            .expect("page 1");
        let second_page = query_clips(&db.pool, None, &[], &[], 2, 2)
            .await
            .expect("page 2");

        assert_eq!(uuids(&first_page), vec!["newest", "middle"]);
        assert_eq!(uuids(&second_page), vec!["oldest"]);
    }

    #[tokio::test]
    async fn search_respects_the_type_filter() {
        let db = test_db().await;
        insert_clip(&db, "a-text", "text", None, "-1 days").await;
        insert_clip(&db, "b-image", "image", None, "-2 days").await;

        // Both rows contain "body of", so only the type filter can separate them.
        let clips = query_clips_matching(
            &db.pool,
            "%body of%",
            None,
            &["image".to_string()],
            &[],
            20,
            0,
        )
        .await
        .expect("search");

        assert_eq!(uuids(&clips), vec!["b-image"]);
    }

    /// A type name is never interpolated into SQL, so a hostile value can only ever be
    /// dropped.
    #[test]
    fn unknown_type_names_are_dropped_rather_than_reaching_sql() {
        let kept = normalise_clip_types(Some(vec![
            "image".to_string(),
            "'; DROP TABLE clips; --".to_string(),
            "nonsense".to_string(),
        ]));

        assert_eq!(kept, vec!["image".to_string()]);
        assert!(normalise_clip_types(None).is_empty());
    }

    #[test]
    fn an_unparseable_folder_id_is_reported() {
        assert_eq!(resolve_folder_filter(None), Ok(None));
        assert_eq!(resolve_folder_filter(Some("7")), Ok(Some(7)));
        assert!(resolve_folder_filter(Some("not-a-number")).is_err());
    }

    /// Sets the source application on an already inserted clip.
    async fn set_source_app(db: &Database, uuid: &str, app: &str) {
        sqlx::query("UPDATE clips SET source_app = ? WHERE uuid = ?")
            .bind(app)
            .bind(uuid)
            .execute(&db.pool)
            .await
            .expect("set source app");
    }

    #[tokio::test]
    async fn the_source_app_filter_narrows_to_that_application() {
        let db = test_db().await;
        insert_clip(&db, "from-chrome", "text", None, "-1 days").await;
        insert_clip(&db, "from-word", "text", None, "-2 days").await;
        set_source_app(&db, "from-chrome", "chrome.exe").await;
        set_source_app(&db, "from-word", "winword.exe").await;

        let clips = query_clips(&db.pool, None, &[], &["chrome.exe".to_string()], 20, 0)
            .await
            .expect("query");

        assert_eq!(uuids(&clips), vec!["from-chrome"]);
    }

    #[tokio::test]
    async fn the_source_app_filter_combines_with_the_type_filter() {
        let db = test_db().await;
        insert_clip(&db, "chrome-image", "image", None, "-1 days").await;
        insert_clip(&db, "chrome-text", "text", None, "-2 days").await;
        insert_clip(&db, "word-image", "image", None, "-3 days").await;
        set_source_app(&db, "chrome-image", "chrome.exe").await;
        set_source_app(&db, "chrome-text", "chrome.exe").await;
        set_source_app(&db, "word-image", "winword.exe").await;

        let clips = query_clips(
            &db.pool,
            None,
            &["image".to_string()],
            &["chrome.exe".to_string()],
            20,
            0,
        )
        .await
        .expect("query");

        assert_eq!(uuids(&clips), vec!["chrome-image"]);
    }

    #[tokio::test]
    async fn the_source_app_list_is_ordered_by_how_much_each_holds() {
        let db = test_db().await;
        insert_clip(&db, "a", "text", None, "-1 days").await;
        insert_clip(&db, "b", "text", None, "-2 days").await;
        insert_clip(&db, "c", "text", None, "-3 days").await;
        insert_clip(&db, "no-app", "text", None, "-4 days").await;
        set_source_app(&db, "a", "chrome.exe").await;
        set_source_app(&db, "b", "chrome.exe").await;
        set_source_app(&db, "c", "winword.exe").await;

        let apps = query_source_apps(&db.pool).await.expect("list source apps");

        assert_eq!(
            apps.iter()
                .map(|app| (app.name.as_str(), app.count))
                .collect::<Vec<_>>(),
            vec![("chrome.exe", 2), ("winword.exe", 1)],
            "ordered by count, and a clip with no source app is left out"
        );
    }

    /// A source application name is arbitrary text, so it is never validated and never
    /// interpolated: it only ever reaches the query as a bound parameter.
    #[test]
    fn source_app_values_are_kept_verbatim_and_bound() {
        let kept = normalise_source_apps(Some(vec![
            "chrome.exe".to_string(),
            "   ".to_string(),
            "'; DROP TABLE clips; --".to_string(),
        ]));

        assert_eq!(
            kept,
            vec!["chrome.exe", "'; DROP TABLE clips; --"],
            "only blank values are dropped"
        );
        assert!(normalise_source_apps(None).is_empty());
    }

    #[test]
    fn the_source_app_filter_is_capped() {
        let many: Vec<String> = (0..200).map(|index| format!("app-{index}.exe")).collect();

        assert_eq!(normalise_source_apps(Some(many)).len(), 64);
    }
}
