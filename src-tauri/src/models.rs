use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::OnceLock;

use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub theme: String,
    pub mica_effect: String,
    pub language: String,
    pub max_items: i64,
    pub auto_delete_days: i64,
    pub hotkey: String,
    pub auto_paste: bool,
    pub paste_method: String,
    pub ignore_ghost_clips: bool,
    pub startup_with_windows: bool,
    pub round_corners: bool,
    pub float_above_taskbar: bool,
    pub card_size: String,

    /// Typography.
    ///
    /// An empty family means "keep what the stylesheet already uses", so the default
    /// look is exactly what it was before these existed. Interface and content are
    /// separate because the clip text is deliberately monospaced, and one global font
    /// would make code clips harder to read.
    pub ui_font_family: String,
    pub ui_font_weight: String,
    pub content_font_family: String,
    pub content_font_weight: String,
    /// One of `small`, `default`, `large`. Applied to the whole interface.
    pub font_scale: String,

    // Privacy
    pub ignored_apps: HashSet<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            mica_effect: "clear".to_string(),
            language: "en".to_string(),
            max_items: 1000,
            auto_delete_days: 30,
            hotkey: "Ctrl+Shift+V".to_string(),
            auto_paste: false,
            paste_method: "shift_insert".to_string(),
            ignore_ghost_clips: false,
            startup_with_windows: false,
            round_corners: false,
            float_above_taskbar: true,
            card_size: "large".to_string(),

            ui_font_family: "".to_string(),
            ui_font_weight: "400".to_string(),
            content_font_family: "".to_string(),
            content_font_weight: "400".to_string(),
            font_scale: "default".to_string(),

            ignored_apps: HashSet::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Clip {
    pub id: i64,
    pub uuid: String,
    pub clip_type: String,
    pub content: Vec<u8>,
    pub text_preview: String,
    pub content_hash: String,
    pub folder_id: Option<i64>,
    pub is_deleted: bool,
    pub is_thumbnail: bool,
    pub source_app: Option<String>,
    pub source_icon: Option<String>,
    pub metadata: Option<String>,
    /// A name the user gave this clip. `None` means fall back to the source application.
    pub title: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_accessed: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Folder {
    pub id: i64,
    pub name: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub is_system: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

pub fn get_runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    if let Some(rt) = RUNTIME.get() {
        return Ok(rt);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;

    RUNTIME.set(rt).ok();
    Ok(RUNTIME.get().unwrap())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub id: String,
    pub clip_type: String,
    pub content: String,
    pub preview: String,
    pub folder_id: Option<String>,
    pub created_at: String,
    pub source_app: Option<String>,
    pub source_icon: Option<String>,
    pub metadata: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderItem {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub is_system: bool,
    pub item_count: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typography_settings_round_trip_through_json() {
        let settings = AppSettings {
            ui_font_family: "Microsoft YaHei".to_string(),
            ui_font_weight: "600".to_string(),
            content_font_family: "Consolas".to_string(),
            content_font_weight: "500".to_string(),
            font_scale: "large".to_string(),
            ..Default::default()
        };

        let json = serde_json::to_string(&settings).expect("serialize");
        let restored: AppSettings = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.ui_font_family, "Microsoft YaHei");
        assert_eq!(restored.ui_font_weight, "600");
        assert_eq!(restored.content_font_family, "Consolas");
        assert_eq!(restored.content_font_weight, "500");
        assert_eq!(restored.font_scale, "large");
    }

    /// A settings file written before the typography fields existed has no keys for
    /// them. It has to keep loading, with the defaults standing in, or upgrading would
    /// break the user's saved configuration.
    #[test]
    fn a_settings_file_from_before_typography_still_loads() {
        let json = r#"{"theme":"dark","mica_effect":"clear"}"#;

        let settings: AppSettings = serde_json::from_str(json).expect("deserialize");

        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.ui_font_family, "");
        assert_eq!(settings.font_scale, "default");
    }

    /// The defaults have to leave the interface looking exactly as it did before the
    /// font settings existed.
    #[test]
    fn the_default_typography_is_the_existing_look() {
        let settings = AppSettings::default();

        assert_eq!(settings.ui_font_family, "");
        assert_eq!(settings.content_font_family, "");
        assert_eq!(settings.ui_font_weight, "400");
        assert_eq!(settings.font_scale, "default");
    }
}
