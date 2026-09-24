use tauri::{AppHandle, Emitter, Listener};
// Import functions directly from the crate root
use crate::database::Database;
#[cfg(target_os = "windows")]
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use clipboard_rs::common::RustImage;
use clipboard_rs::{Clipboard, ClipboardContext};
use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
#[cfg(target_os = "windows")]
use std::ffi::OsStr;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
use std::sync::{Arc, OnceLock};
use tauri_plugin_clipboard_x::{read_text, start_listening};
use uuid::Uuid;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::MAX_PATH;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
#[cfg(target_os = "windows")]
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    GetObjectW, ReleaseDC, SelectObject, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, HBITMAP,
};
#[cfg(target_os = "windows")]
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardOwner,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
#[cfg(target_os = "windows")]
use windows::Win32::System::ProcessStatus::{GetModuleBaseNameW, GetModuleFileNameExW};
#[cfg(target_os = "windows")]
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY, VK_CONTROL, VK_INSERT, VK_SHIFT,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Shell::{
    SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyIcon, DrawIconEx, GetForegroundWindow, GetIconInfo, GetWindowThreadProcessId, DI_NORMAL,
    ICONINFO,
};

// GLOBAL STATE: Store the hash of the clip we just pasted ourselves.
// If the next clipboard change matches this hash, we ignore it (don't update timestamp).
static IGNORE_HASH: Lazy<parking_lot::Mutex<Option<String>>> =
    Lazy::new(|| parking_lot::Mutex::new(None));
static LAST_STABLE_HASH: Lazy<parking_lot::Mutex<Option<String>>> =
    Lazy::new(|| parking_lot::Mutex::new(None));
pub static CLIPBOARD_SYNC: Lazy<Arc<tokio::sync::Mutex<()>>> =
    Lazy::new(|| Arc::new(tokio::sync::Mutex::new(())));

use std::sync::atomic::{AtomicU64, Ordering};
static DEBOUNCE_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn set_ignore_hash(hash: String) {
    let mut lock = IGNORE_HASH.lock();
    *lock = Some(hash);
}

pub fn init(app: &AppHandle, db: Arc<Database>) {
    let app_clone = app.clone();
    let db_clone = db.clone();

    // Start monitor
    // tauri-plugin-clipboard-x exposes start_listening(app_handle)
    // It returns impl Future, so we need to spawn it or block.
    // Since init is synchronous here, we spawn it.
    let app_for_start = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = start_listening(app_for_start).await {
            log::error!("CLIPBOARD: Failed to start listener: {}", e);
        }
    });

    // Listen to clipboard changes
    // The event name found in source code: "plugin:clipboard-x://clipboard_changed"
    let event_name = "plugin:clipboard-x://clipboard_changed";

    app.listen(event_name, move |_event| {
        let app = app_clone.clone();
        let db = db_clone.clone();

        // Capture source app info IMMEDIATELY at event time, before debounce delay.
        // If we wait until after the delay, the user may have already switched to PastePaw,
        // causing frontmostApplication to return our own app instead of the real source.
        let source_app_info = get_clipboard_owner_app_info();

        // DEBOUNCE LOGIC:
        let current_count = DEBOUNCE_COUNTER.fetch_add(1, Ordering::SeqCst) + 1;

        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;

            if DEBOUNCE_COUNTER.load(Ordering::SeqCst) != current_count {
                log::debug!(
                    "CLIPBOARD: Debounce: Aborting older event, current_count:{}",
                    current_count
                );
                return;
            }

            process_clipboard_change(app, db, source_app_info).await;
        });
    });
}

type SourceAppInfo = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    bool,
);

struct ClipboardImageRead {
    png_bytes: Vec<u8>,
    width: u32,
    height: u32,
    raw_hash: String,
    decode_ms: u128,
    source_type: &'static str,
}

fn read_clipboard_image_with_clipboard_rs(
    source_type: &'static str,
) -> Result<ClipboardImageRead, String> {
    let ctx = ClipboardContext::new().map_err(|e| e.to_string())?;
    let image = ctx.get_image().map_err(|e| e.to_string())?;
    let (width, height) = image.get_size();

    let dynamic_image = image.get_dynamic_image().map_err(|e| e.to_string())?;
    let raw_hash = calculate_hash(dynamic_image.as_bytes());

    let png_bytes = image
        .to_png()
        .map_err(|e| e.to_string())?
        .get_bytes()
        .to_vec();

    Ok(ClipboardImageRead {
        png_bytes,
        width,
        height,
        raw_hash,
        decode_ms: 0,
        source_type,
    })
}

fn read_clipboard_image_fast() -> Result<ClipboardImageRead, String> {
    read_clipboard_image_with_clipboard_rs("clipboard-rs-image")
}

/// The rich formats one clipboard change carries, beyond plain text and images.
#[derive(Default)]
struct CapturedFormats {
    files: Vec<String>,
    html: Option<String>,
    rtf: Option<Vec<u8>>,
}

/// `#define CF_HDROP 15`. Hard-coded rather than imported because the `windows` crate
/// exposes the clipboard format constants behind its Ole feature, which is not enabled.
const CF_HDROP: u32 = 15;

/// Closes the clipboard on drop, so no early return can leave it locked and wedge
/// every other application that touches the clipboard.
struct ClipboardSession;

impl ClipboardSession {
    fn open() -> Result<Self, String> {
        unsafe { OpenClipboard(None) }.map_err(|e| format!("OpenClipboard failed: {e}"))?;
        Ok(Self)
    }
}

impl Drop for ClipboardSession {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// Unlocks a `GlobalLock`ed handle on drop.
struct GlobalLockGuard(HGLOBAL);

impl Drop for GlobalLockGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalUnlock(self.0);
        }
    }
}

/// Reads the clipboard's rich formats, if any.
///
/// Everything is read in a single clipboard session. Opening per format would mean
/// repeated open/close pairs and, worse, a window in which another application could
/// swap the contents between two reads.
fn read_rich_clipboard_formats() -> CapturedFormats {
    let mut captured = CapturedFormats::default();

    let Ok(_session) = ClipboardSession::open() else {
        log::warn!("CLIPBOARD: could not open the clipboard to read rich formats");
        return captured;
    };

    if let Some(bytes) = read_locked_clipboard_format(CF_HDROP) {
        match crate::clipboard_formats::parse_hdrop(&bytes) {
            Ok(paths) => captured.files = paths,
            Err(e) => log::warn!("CLIPBOARD: could not parse CF_HDROP: {e:?}"),
        }
    }

    // What gets stored is the fragment, not the raw CF_HTML payload: that payload
    // embeds byte offsets describing its own header, which would be wrong the moment
    // anything re-encoded it. It is rebuilt by `build_cf_html` on the way out.
    if let Some(bytes) = read_locked_clipboard_format(html_clipboard_format()) {
        match crate::clipboard_formats::parse_cf_html(&bytes) {
            Some(fragment) => captured.html = Some(fragment),
            None => log::warn!("CLIPBOARD: could not parse a CF_HTML payload"),
        }
    }

    // RTF has no header to normalise, so it is kept exactly as it arrived.
    if let Some(bytes) = read_locked_clipboard_format(rtf_clipboard_format()) {
        captured.rtf = Some(bytes);
    }

    captured
}

/// Reads a single clipboard format while the clipboard is already open.
fn read_locked_clipboard_format(format: u32) -> Option<Vec<u8>> {
    if unsafe { IsClipboardFormatAvailable(format) }.is_err() {
        return None;
    }

    let handle = unsafe { GetClipboardData(format) }.ok()?;
    let hglobal = HGLOBAL(handle.0);

    let ptr = unsafe { GlobalLock(hglobal) } as *const u8;
    if ptr.is_null() {
        log::warn!("CLIPBOARD: GlobalLock for format {format} returned null");
        return None;
    }
    let _lock = GlobalLockGuard(hglobal);

    let size = unsafe { GlobalSize(hglobal) };
    if size == 0 {
        return None;
    }
    // SAFETY: the handle stays locked for as long as `_lock` is alive, and GlobalSize
    // reports the size of the allocation the clipboard owns.
    Some(unsafe { std::slice::from_raw_parts(ptr, size) }.to_vec())
}

/// `#define CF_DIB 8`.
const CF_DIB: u32 = 8;
/// `#define CF_UNICODETEXT 13`.
const CF_UNICODETEXT: u32 = 13;
/// `#define CF_DIBV5 17`.
const CF_DIBV5: u32 = 17;

/// The formats to place on the clipboard for one clip.
///
/// Several are written at once on purpose: the target application picks whichever
/// it understands, which is what lets a clip paste back in the shape it was copied
/// in rather than always degrading to plain text.
#[derive(Default)]
pub struct ClipboardPayload {
    pub text: Option<String>,
    pub files: Option<Vec<String>>,
    /// The PNG bytes stored for an image clip.
    pub image_png: Option<Vec<u8>>,
    /// An HTML fragment, re-encoded into a `CF_HTML` payload on the way out.
    pub html: Option<String>,
    /// Raw RTF bytes, written back unchanged.
    pub rtf: Option<Vec<u8>>,
}

impl ClipboardPayload {
    pub fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.files.as_ref().is_none_or(|files| files.is_empty())
            && self.image_png.is_none()
            && self.html.is_none()
            && self.rtf.is_none()
    }
}

/// Replaces the clipboard contents with `payload`.
///
/// The caller is responsible for having taken `CLIPBOARD_SYNC` and for having
/// stopped the clipboard monitor, so that this replacement is not captured back
/// as a new clip.
pub fn write_clipboard_payload(payload: &ClipboardPayload) -> Result<(), String> {
    if payload.is_empty() {
        return Err("clip has no clipboard formats to write".to_string());
    }

    let _session = ClipboardSession::open()?;
    // Emptying first is what makes this a replacement. Without it the previous
    // clipboard contents would remain reachable under the formats we do not set.
    unsafe { EmptyClipboard() }.map_err(|e| format!("EmptyClipboard failed: {e}"))?;

    if let Some(files) = payload.files.as_ref().filter(|files| !files.is_empty()) {
        set_clipboard_bytes(CF_HDROP, &crate::clipboard_formats::build_hdrop(files))?;
    }
    if let Some(png) = payload.image_png.as_deref() {
        write_image_formats(png)?;
    }
    // Rich text goes on before plain text: an application that understands both then
    // has the formatted version available to prefer.
    if let Some(html) = payload.html.as_deref() {
        set_clipboard_bytes(
            html_clipboard_format(),
            &crate::clipboard_formats::build_cf_html(html, None),
        )?;
    }
    if let Some(rtf) = payload.rtf.as_deref() {
        set_clipboard_bytes(rtf_clipboard_format(), rtf)?;
    }
    if let Some(text) = payload.text.as_deref() {
        set_clipboard_bytes(CF_UNICODETEXT, &to_utf16_bytes(text))?;
    }

    Ok(())
}

/// Registers a clipboard format by name, or returns the existing id if some other
/// process already registered it.
fn register_clipboard_format(name: &str) -> u32 {
    let mut wide: Vec<u16> = name.encode_utf16().collect();
    wide.push(0);
    unsafe { RegisterClipboardFormatW(windows::core::PCWSTR(wide.as_ptr())) }
}

/// The id of the registered `"PNG"` format. Registration is idempotent and the id
/// stays valid for the process lifetime, so it is resolved once.
fn png_clipboard_format() -> u32 {
    static FORMAT: OnceLock<u32> = OnceLock::new();
    *FORMAT.get_or_init(|| register_clipboard_format("PNG"))
}

/// The id of the registered `"HTML Format"` format, i.e. `CF_HTML`.
fn html_clipboard_format() -> u32 {
    static FORMAT: OnceLock<u32> = OnceLock::new();
    *FORMAT.get_or_init(|| register_clipboard_format("HTML Format"))
}

/// The id of the registered `"Rich Text Format"` format, i.e. `CF_RTF`.
///
/// `CF_RTF` has no predefined constant; unlike `CF_DIB` and friends it only exists
/// as a registered name.
fn rtf_clipboard_format() -> u32 {
    static FORMAT: OnceLock<u32> = OnceLock::new();
    *FORMAT.get_or_init(|| register_clipboard_format("Rich Text Format"))
}

/// Puts an image on the clipboard in the formats applications actually look for.
///
/// Three go on at once: the untouched PNG bytes for browsers and anything PNG aware,
/// `CF_DIBV5` for applications that honour the alpha channel, and the older `CF_DIB`
/// for applications that only search for that. Writing the image here instead of
/// from the WebView is what lets transparency survive a paste; the previous
/// `navigator.clipboard` path only ever produced an opaque bitmap.
fn write_image_formats(png_bytes: &[u8]) -> Result<(), String> {
    let decoded = image::load_from_memory_with_format(png_bytes, image::ImageFormat::Png)
        .map_err(|e| format!("decoding the stored image failed: {e}"))?
        .to_rgba8();
    let (width, height) = (decoded.width(), decoded.height());
    let rgba = decoded.into_raw();

    let dibv5 = crate::clipboard_formats::build_dibv5(&rgba, width, height)
        .map_err(|e| format!("building CF_DIBV5 failed: {e:?}"))?;
    let dib = crate::clipboard_formats::build_dib(&rgba, width, height)
        .map_err(|e| format!("building CF_DIB failed: {e:?}"))?;

    set_clipboard_bytes(CF_DIBV5, &dibv5)?;
    set_clipboard_bytes(CF_DIB, &dib)?;
    set_clipboard_bytes(png_clipboard_format(), png_bytes)
}

/// UTF-16LE followed by a terminating NUL, the layout `CF_UNICODETEXT` requires.
fn to_utf16_bytes(text: &str) -> Vec<u8> {
    let mut out: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// Copies `bytes` into a movable global allocation and hands the allocation to the
/// clipboard.
fn set_clipboard_bytes(format: u32, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() {
        return Err(format!(
            "refusing to set format {format} to an empty payload"
        ));
    }

    let hglobal = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }
        .map_err(|e| format!("GlobalAlloc for format {format} failed: {e}"))?;

    let dest = unsafe { GlobalLock(hglobal) } as *mut u8;
    if dest.is_null() {
        unsafe {
            let _ = GlobalFree(Some(hglobal));
        }
        return Err(format!("GlobalLock for format {format} returned null"));
    }
    // SAFETY: the allocation is exactly `bytes.len()` long and stays locked until the
    // unlock below, and the two buffers cannot overlap.
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest, bytes.len()) };
    unsafe {
        let _ = GlobalUnlock(hglobal);
    }

    // Ownership transfers to the clipboard only on success, so on failure the handle
    // is still ours and has to be freed or it leaks.
    match unsafe { SetClipboardData(format, Some(HANDLE(hglobal.0))) } {
        Ok(_) => Ok(()),
        Err(e) => {
            unsafe {
                let _ = GlobalFree(Some(hglobal));
            }
            Err(format!("SetClipboardData for format {format} failed: {e}"))
        }
    }
}

async fn process_clipboard_change(
    app: AppHandle,
    db: Arc<Database>,
    source_app_info: SourceAppInfo,
) {
    let started = std::time::Instant::now();
    let mut image_read_ms = 0u128;
    let mut image_decode_ms = 0u128;
    let mut text_read_ms = 0u128;
    let mut was_existing = false;
    let _guard = CLIPBOARD_SYNC.lock().await;

    let mut clip_type = "text";
    let mut clip_content = Vec::new();
    let mut full_image_content: Option<Vec<u8>> = None;
    let mut clip_preview = String::new();
    let mut clip_hash = String::new();
    let mut metadata = String::new();
    let mut found_content = false;

    // Try Image (in-memory path, no temp file write).
    log::debug!("CLIPBOARD: Attempting to read image from clipboard");
    let image_read_started = std::time::Instant::now();
    if let Ok(read_image_result) = read_clipboard_image_fast() {
        image_read_ms = image_read_started.elapsed().as_millis();
        log::debug!(
            "CLIPBOARD: Image read successfully, source_type={}, takes {} ms",
            read_image_result.source_type,
            image_read_ms
        );

        let bytes = read_image_result.png_bytes;
        let width = read_image_result.width;
        let height = read_image_result.height;
        image_decode_ms = read_image_result.decode_ms;
        let size_bytes = bytes.len();
        clip_hash = read_image_result.raw_hash;
        clip_content = Vec::new();
        full_image_content = Some(bytes);
        clip_type = "image";
        clip_preview = "[Image]".to_string();
        metadata = serde_json::json!({
            "width": width,
            "height": height,
            "format": "png",
            "size_bytes": size_bytes
        })
        .to_string();
        found_content = true;
        log::debug!(
            "CLIPBOARD: Found image: {}x{}, source_type={}, png_bytes={}",
            width,
            height,
            read_image_result.source_type,
            size_bytes
        );
    }

    // Files and rich text are read in one clipboard session, and only when the
    // clipboard holds no image, so the image path does no extra work.
    let rich = if found_content {
        CapturedFormats::default()
    } else {
        read_rich_clipboard_formats()
    };

    // Files are checked before text on purpose: copying a file in Explorer also puts
    // a text rendition on the clipboard, so a text-first check would record the file
    // name as a text clip and lose the file itself.
    if !found_content && !rich.files.is_empty() {
        log::debug!("CLIPBOARD: Found {} file(s)", rich.files.len());
        let joined = rich.files.join("\n");
        clip_content = joined.into_bytes();
        clip_hash = calculate_hash(&clip_content);
        clip_preview = crate::clipboard_formats::describe_files(&rich.files);
        clip_type = "file";
        found_content = true;
    }

    if !found_content {
        // Try Text
        let text_read_started = std::time::Instant::now();
        if let Ok(text) = read_text().await {
            text_read_ms = text_read_started.elapsed().as_millis();
            let text = text.trim();
            if !text.is_empty() {
                clip_content = text.as_bytes().to_vec();
                clip_hash = calculate_hash(&clip_content);
                clip_type = "text";
                clip_preview = text.chars().take(200).collect::<String>();
                found_content = true;
                log::debug!("CLIPBOARD: Found text: {}", clip_preview);
            }
        }
    }

    if !found_content {
        return;
    }

    // Stable Hash Check
    {
        let mut lock = LAST_STABLE_HASH.lock();
        if let Some(ref last_hash) = *lock {
            if last_hash == &clip_hash {
                return;
            }
        }
        *lock = Some(clip_hash.clone());
    }

    // Check ignore self-paste
    {
        let mut lock = IGNORE_HASH.lock();
        if let Some(ignore_hash) = lock.take() {
            if ignore_hash == clip_hash {
                log::info!(
                    "CLIPBOARD: Detected self-paste for hash {}, proceeding to update timestamp",
                    ignore_hash
                );
            }
        }
    }

    // Source app info was captured at event time (before debounce) to avoid race conditions
    let (source_app, source_icon, exe_name, full_path, is_explicit_owner) = source_app_info;
    log::info!(
        "CLIPBOARD: Source app: {:?}, exe_name: {:?}, full_path: {:?}, explicit: {}",
        source_app,
        exe_name,
        full_path,
        is_explicit_owner
    );

    // Check settings (cached via SettingsManager)
    use crate::settings_manager::SettingsManager;
    use tauri::Manager;
    let manager = app.state::<Arc<SettingsManager>>();
    let settings = manager.get();

    // Copied out as plain numbers so they stay available after `settings` has been
    // used further down for the ignore list.
    let retention = (settings.auto_delete_days, settings.max_items);

    if settings.ignore_ghost_clips && !is_explicit_owner {
        log::info!("CLIPBOARD: Ignoring ghost clip (unknown owner)");
        return;
    }

    // Check if the app is in the ignore list (Case Insensitive)
    let is_ignored = |name: &str| {
        let name_lower = name.to_lowercase();
        settings
            .ignored_apps
            .iter()
            .any(|app| app.to_lowercase() == name_lower)
    };

    if let Some(ref path) = full_path {
        if is_ignored(path) {
            log::info!(
                "CLIPBOARD: Ignoring content from ignored app (path match): {}",
                path
            );
            return;
        }
    }

    if let Some(ref exe) = exe_name {
        if is_ignored(exe) {
            log::info!(
                "CLIPBOARD: Ignoring content from ignored app (exe match): {}",
                exe
            );
            return;
        }
    }

    // DB Logic
    let pool = &db.pool;

    let db_lookup_started = std::time::Instant::now();
    let existing_uuid: Option<String> =
        sqlx::query_scalar::<_, String>(r#"SELECT uuid FROM clips WHERE content_hash = ?"#)
            .bind(&clip_hash)
            .fetch_optional(pool)
            .await
            .unwrap_or(None);
    let db_lookup_ms = db_lookup_started.elapsed().as_millis();

    let db_write_started = std::time::Instant::now();
    let emitted_id = if let Some(existing_id) = existing_uuid {
        was_existing = true;
        if clip_type == "image" {
            let _ = sqlx::query(
                r#"
                UPDATE clips
                SET created_at = CURRENT_TIMESTAMP,
                    is_deleted = 0,
                    source_app = ?,
                    source_icon = ?,
                    content = ?,
                    text_preview = ?,
                    metadata = ?,
                    is_thumbnail = 0
                WHERE uuid = ?
                "#,
            )
            .bind(&source_app)
            .bind(&source_icon)
            .bind(&clip_content)
            .bind(&clip_preview)
            .bind(Some(metadata.clone()))
            .bind(&existing_id)
            .execute(pool)
            .await;

            if let Some(full_bytes) = &full_image_content {
                match persist_full_image_file(&existing_id, full_bytes) {
                    Ok(file_path) => {
                        let _ = sqlx::query(
                            r#"
                            INSERT OR REPLACE INTO clip_images (clip_uuid, full_content, file_path, file_size, storage_kind, mime_type, created_at)
                            VALUES (?, x'', ?, ?, 'file', 'image/png', CURRENT_TIMESTAMP)
                            "#,
                        )
                        .bind(&existing_id)
                        .bind(&file_path)
                        .bind(full_bytes.len() as i64)
                        .execute(pool)
                        .await;
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to persist full image file for existing clip {}: {}",
                            existing_id,
                            e
                        );
                    }
                }
            }
        } else {
            let _ = sqlx::query(r#"UPDATE clips SET created_at = CURRENT_TIMESTAMP, is_deleted = 0, source_app = ?, source_icon = ? WHERE uuid = ?"#)
                .bind(&source_app)
                .bind(&source_icon)
                .bind(&existing_id)
                .execute(pool)
                .await;
        }
        existing_id
    } else {
        let clip_uuid = Uuid::new_v4().to_string();

        let _ = sqlx::query(
            r#"
            INSERT INTO clips (uuid, clip_type, content, text_preview, content_hash, folder_id, is_deleted, is_thumbnail, source_app, source_icon, metadata, created_at, last_accessed)
            VALUES (?, ?, ?, ?, ?, NULL, 0, ?, ?, ?, ?, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            "#,
        )
        .bind(&clip_uuid)
        .bind(clip_type)
        .bind(&clip_content)
        .bind(&clip_preview)
        .bind(&clip_hash)
        .bind(false)
        .bind(&source_app)
        .bind(&source_icon)
        .bind(if clip_type == "image" {
            Some(metadata)
        } else {
            None
        })
        .execute(pool)
        .await;

        if clip_type == "image" {
            if let Some(full_bytes) = &full_image_content {
                match persist_full_image_file(&clip_uuid, full_bytes) {
                    Ok(file_path) => {
                        let _ = sqlx::query(
                            r#"
                            INSERT OR REPLACE INTO clip_images (clip_uuid, full_content, file_path, file_size, storage_kind, mime_type, created_at)
                            VALUES (?, x'', ?, ?, 'file', 'image/png', CURRENT_TIMESTAMP)
                            "#,
                        )
                        .bind(&clip_uuid)
                        .bind(&file_path)
                        .bind(full_bytes.len() as i64)
                        .execute(pool)
                        .await;
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to persist full image file for new clip {}, dropping clip: {}",
                            clip_uuid,
                            e
                        );
                        let _ = sqlx::query(r#"DELETE FROM clips WHERE uuid = ?"#)
                            .bind(&clip_uuid)
                            .execute(pool)
                            .await;
                        return;
                    }
                }
            }
        }
        clip_uuid
    };
    let db_write_ms = db_write_started.elapsed().as_millis();

    // Anything rich that arrived alongside a plain text clip rides with it, so pasting
    // into a word processor keeps its formatting instead of degrading to plain text.
    let mut extra_formats: Vec<(&'static str, Vec<u8>)> = Vec::new();
    if !rich.files.is_empty() {
        match serde_json::to_vec(&rich.files) {
            Ok(payload) => extra_formats.push((crate::clipboard_formats::FORMAT_FILE, payload)),
            Err(e) => log::error!("CLIPBOARD: could not encode the file list: {e}"),
        }
    }
    if clip_type == "text" {
        if let Some(html) = rich.html {
            extra_formats.push((crate::clipboard_formats::FORMAT_HTML, html.into_bytes()));
        }
        if let Some(rtf) = rich.rtf {
            extra_formats.push((crate::clipboard_formats::FORMAT_RTF, rtf));
        }
    }

    // Persisted once the clip row exists so the foreign key holds, and on both the
    // insert and the update path.
    for (format, payload_bytes) in &extra_formats {
        if let Err(e) = sqlx::query(
            r#"INSERT OR REPLACE INTO clip_formats (clip_uuid, format, content)
               VALUES (?, ?, ?)"#,
        )
        .bind(&emitted_id)
        .bind(format)
        .bind(payload_bytes)
        .execute(pool)
        .await
        {
            log::error!("Failed to persist format {format} for clip {emitted_id}: {e}");
        }
    }

    let emit_started = std::time::Instant::now();
    let _ = app.emit(
        "clipboard-change",
        &serde_json::json!({
            "id": emitted_id,
            "content": clip_preview,
            "clip_type": clip_type,
            "source_app": source_app,
            "source_icon": source_icon,
            "created_at": chrono::Utc::now().to_rfc3339()
        }),
    );
    let emit_ms = emit_started.elapsed().as_millis();

    // Retention runs on every ingest so the history cannot drift past its limit during
    // a session. The guard inside `prune` keeps this cheap when nothing is over.
    match crate::retention::prune(pool, retention.0, retention.1).await {
        Ok(report) if report.total() > 0 => {
            log::info!("Retention: removed {} clip(s)", report.total());
            // The window still has the pruned clips in its list.
            let _ = app.emit("clipboard-change", ());
        }
        Ok(_) => {}
        Err(e) => log::error!("Retention failed: {e}"),
    }

    log::info!(
        "[perf][clipboard_ingest] type={} existing={} full_bytes={} thumb_bytes={} image_read_ms={} decode_ms={} text_read_ms={} db_lookup_ms={} db_write_ms={} emit_ms={} total_ms={}",
        clip_type,
        was_existing,
        full_image_content.as_ref().map(|v| v.len()).unwrap_or(0),
        if clip_type == "image" { clip_content.len() } else { 0 },
        image_read_ms,
        image_decode_ms,
        text_read_ms,
        db_lookup_ms,
        db_write_ms,
        emit_ms,
        started.elapsed().as_millis()
    );
}
fn calculate_hash(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    let result = hasher.finalize();
    format!("{:x}", result)
}

fn get_image_store_dir() -> std::path::PathBuf {
    let current_dir = std::env::current_dir().unwrap_or(std::path::PathBuf::from("."));
    let app_data_dir = match dirs::data_dir() {
        Some(path) => path.join("PastePaw"),
        None => current_dir.join("PastePaw"),
    };
    app_data_dir.join("images")
}

pub fn persist_full_image_file(clip_uuid: &str, png_bytes: &[u8]) -> Result<String, String> {
    let dir = get_image_store_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file_path = dir.join(format!("{}.png", clip_uuid));
    std::fs::write(&file_path, png_bytes).map_err(|e| e.to_string())?;
    Ok(file_path.to_string_lossy().to_string())
}

pub fn read_full_image_file(file_path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(file_path).map_err(|e| e.to_string())
}

pub fn remove_full_image_file(file_path: &str) {
    if let Err(e) = std::fs::remove_file(file_path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            log::warn!("Failed to delete image file {}: {}", file_path, e);
        }
    }
}

#[cfg(target_os = "windows")]
fn get_clipboard_owner_app_info() -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    bool,
) {
    unsafe {
        let (hwnd, is_explicit) = match GetClipboardOwner() {
            Ok(h) if !h.0.is_null() => (h, true),
            Err(e) => {
                log::info!(
                    "CLIPBOARD: GetClipboardOwner failed: {:?}, falling back to foreground window",
                    e
                );
                (GetForegroundWindow(), false)
            }
            Ok(_) => {
                log::info!(
                    "CLIPBOARD: GetClipboardOwner returned null, falling back to foreground window"
                );
                (GetForegroundWindow(), false)
            }
        };

        if hwnd.0.is_null() {
            return (None, None, None, None, false);
        }

        let mut process_id = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));

        if process_id == 0 {
            return (None, None, None, None, false);
        }

        let process_handle = match OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
            false,
            process_id,
        ) {
            Ok(h) => h,
            Err(_) => return (None, None, None, None, false),
        };

        let mut name_buffer = [0u16; MAX_PATH as usize];
        let name_size = GetModuleBaseNameW(process_handle, None, &mut name_buffer);
        let exe_name = if name_size > 0 {
            String::from_utf16_lossy(&name_buffer[..name_size as usize])
        } else {
            String::new()
        };

        let mut path_buffer = [0u16; MAX_PATH as usize];
        let path_size = GetModuleFileNameExW(Some(process_handle), None, &mut path_buffer);
        let (app_name, app_icon, full_path) = if path_size > 0 {
            let full_path_str = String::from_utf16_lossy(&path_buffer[..path_size as usize]);

            let desc = get_app_description(&full_path_str);
            let final_name = if let Some(d) = desc {
                Some(d)
            } else {
                if !exe_name.is_empty() {
                    Some(exe_name.clone())
                } else {
                    None
                }
            };

            let icon = extract_icon(&full_path_str);
            (final_name, icon, Some(full_path_str))
        } else {
            (
                if !exe_name.is_empty() {
                    Some(exe_name.clone())
                } else {
                    None
                },
                None,
                None,
            )
        };

        let exe_val = if !exe_name.is_empty() {
            Some(exe_name)
        } else {
            None
        };
        (app_name, app_icon, exe_val, full_path, is_explicit)
    }
}

#[cfg(target_os = "windows")]
unsafe fn get_app_description(path: &str) -> Option<String> {
    unsafe {
        use std::ffi::c_void;

        let wide_path: Vec<u16> = OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let size = GetFileVersionInfoSizeW(windows::core::PCWSTR(wide_path.as_ptr()), None);
        if size == 0 {
            return None;
        }

        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(
            windows::core::PCWSTR(wide_path.as_ptr()),
            Some(0),
            size,
            data.as_mut_ptr() as *mut _,
        )
        .is_err()
        {
            return None;
        }

        let mut lang_ptr: *mut c_void = std::ptr::null_mut();
        let mut lang_len: u32 = 0;

        let translation_query = OsStr::new("\\VarFileInfo\\Translation")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>();

        if !VerQueryValueW(
            data.as_ptr() as *const _,
            windows::core::PCWSTR(translation_query.as_ptr()),
            &mut lang_ptr,
            &mut lang_len,
        )
        .as_bool()
        {
            return None;
        }

        if lang_len < 4 {
            return None;
        }

        let pairs = std::slice::from_raw_parts(lang_ptr as *const u16, (lang_len / 2) as usize);
        let num_pairs = (lang_len / 4) as usize;

        let mut lang_code = pairs[0];
        let mut charset_code = pairs[1];

        for i in 0..num_pairs {
            let code = pairs[i * 2];
            let charset = pairs[i * 2 + 1];

            if code == 0x0804 {
                lang_code = code;
                charset_code = charset;
            }
        }

        let keys = ["FileDescription", "ProductName"];

        for key in keys {
            let query_str = format!(
                "\\StringFileInfo\\{:04x}{:04x}\\{}",
                lang_code, charset_code, key
            );
            let query = OsStr::new(&query_str)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<u16>>();

            let mut desc_ptr: *mut c_void = std::ptr::null_mut();
            let mut desc_len: u32 = 0;

            if VerQueryValueW(
                data.as_ptr() as *const _,
                windows::core::PCWSTR(query.as_ptr()),
                &mut desc_ptr,
                &mut desc_len,
            )
            .as_bool()
            {
                let desc = std::slice::from_raw_parts(desc_ptr as *const u16, desc_len as usize);
                let len = if desc.last() == Some(&0) {
                    desc.len() - 1
                } else {
                    desc.len()
                };
                if len > 0 {
                    return Some(String::from_utf16_lossy(&desc[..len]));
                }
            }
        }

        None
    }
}

#[cfg(target_os = "windows")]
unsafe fn extract_icon(path: &str) -> Option<String> {
    unsafe {
        use image::ImageEncoder;

        let wide_path: Vec<u16> = OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut shfi = SHFILEINFOW::default();

        SHGetFileInfoW(
            windows::core::PCWSTR(wide_path.as_ptr()),
            windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL,
            Some(&mut shfi as *mut _),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
        );

        if shfi.hIcon.is_invalid() {
            return None;
        }

        let icon = shfi.hIcon;
        struct IconGuard(windows::Win32::UI::WindowsAndMessaging::HICON);
        impl Drop for IconGuard {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyIcon(self.0);
                }
            }
        }
        let _guard = IconGuard(icon);

        let mut icon_info = ICONINFO::default();
        if GetIconInfo(icon, &mut icon_info).is_err() {
            return None;
        }

        struct BitmapGuard(HBITMAP);
        impl Drop for BitmapGuard {
            fn drop(&mut self) {
                unsafe {
                    if !self.0.is_invalid() {
                        let _ = DeleteObject(self.0.into());
                    }
                }
            }
        }
        let _bm_mask = BitmapGuard(icon_info.hbmMask);
        let _bm_color = BitmapGuard(icon_info.hbmColor);

        let mut bm = BITMAP::default();
        if GetObjectW(
            icon_info.hbmMask.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        ) == 0
        {
            return None;
        }

        let width = bm.bmWidth;
        let height = if !icon_info.hbmColor.is_invalid() {
            bm.bmHeight
        } else {
            bm.bmHeight / 2
        };

        let screen_dc = GetDC(None);
        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        let mem_bm = CreateCompatibleBitmap(screen_dc, width, height);

        let old_obj = SelectObject(mem_dc, mem_bm.into());

        let _ = DrawIconEx(mem_dc, 0, 0, icon, width, height, 0, None, DI_NORMAL);

        let bi = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };

        let mut pixels = vec![0u8; (width * height * 4) as usize];

        GetDIBits(
            mem_dc,
            mem_bm,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut _),
            &mut BITMAPINFO {
                bmiHeader: bi,
                ..Default::default()
            },
            DIB_RGB_COLORS,
        );

        SelectObject(mem_dc, old_obj);
        let _ = DeleteDC(mem_dc);
        let _ = DeleteObject(mem_bm.into());
        let _ = ReleaseDC(None, screen_dc);

        for chunk in pixels.chunks_exact_mut(4) {
            let b = chunk[0];
            let r = chunk[2];
            chunk[0] = r;
            chunk[2] = b;
        }

        let mut png_data = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png_data);
        encoder
            .write_image(
                &pixels,
                width as u32,
                height as u32,
                image::ColorType::Rgba8,
            )
            .ok()?;

        Some(BASE64.encode(&png_data))
    }
}

#[cfg(target_os = "windows")]
pub fn send_paste_input(method: &str) {
    unsafe {
        let inputs = if method == "ctrl_v" {
            log::info!("send_paste_input: sending Ctrl+V");
            let ctrl_scan = MapVirtualKeyW(VK_CONTROL.0 as u32, MAPVK_VK_TO_VSC) as u16;
            let v_scan = MapVirtualKeyW(0x56, MAPVK_VK_TO_VSC) as u16;
            vec![
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_CONTROL,
                            wScan: ctrl_scan,
                            ..Default::default()
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0x56),
                            wScan: v_scan,
                            ..Default::default()
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0x56),
                            wScan: v_scan,
                            dwFlags: KEYEVENTF_KEYUP,
                            ..Default::default()
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_CONTROL,
                            wScan: ctrl_scan,
                            dwFlags: KEYEVENTF_KEYUP,
                            ..Default::default()
                        },
                    },
                },
            ]
        } else {
            log::info!("send_paste_input: sending Shift+Insert (with KEYEVENTF_EXTENDEDKEY)");
            let shift_scan = MapVirtualKeyW(VK_SHIFT.0 as u32, MAPVK_VK_TO_VSC) as u16;
            let insert_scan = MapVirtualKeyW(VK_INSERT.0 as u32, MAPVK_VK_TO_VSC) as u16;
            vec![
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_SHIFT,
                            wScan: shift_scan,
                            ..Default::default()
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_INSERT,
                            wScan: insert_scan,
                            dwFlags: KEYEVENTF_EXTENDEDKEY,
                            ..Default::default()
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_INSERT,
                            wScan: insert_scan,
                            dwFlags: KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP,
                            ..Default::default()
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_SHIFT,
                            wScan: shift_scan,
                            dwFlags: KEYEVENTF_KEYUP,
                            ..Default::default()
                        },
                    },
                },
            ]
        };

        let result = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        log::info!("send_paste_input: SendInput returned {}", result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_bytes_are_little_endian_and_nul_terminated() {
        assert_eq!(to_utf16_bytes("A"), vec![0x41, 0x00, 0x00, 0x00]);
    }

    /// Anything above U+FFFF has to become a surrogate pair, or the text pastes back
    /// mangled.
    #[test]
    fn utf16_bytes_encode_non_bmp_characters_as_surrogate_pairs() {
        // U+1F600 -> D83D DE00
        assert_eq!(
            to_utf16_bytes("\u{1F600}"),
            vec![0x3D, 0xD8, 0x00, 0xDE, 0x00, 0x00]
        );
    }

    #[test]
    fn utf16_bytes_of_empty_text_is_just_the_terminator() {
        assert_eq!(to_utf16_bytes(""), vec![0x00, 0x00]);
    }

    #[test]
    fn a_payload_counts_as_empty_only_when_it_has_nothing_to_write() {
        assert!(ClipboardPayload::default().is_empty());
        assert!(ClipboardPayload {
            files: Some(Vec::new()),
            ..Default::default()
        }
        .is_empty());
        assert!(!ClipboardPayload {
            text: Some(String::new()),
            ..Default::default()
        }
        .is_empty());
        assert!(!ClipboardPayload {
            files: Some(vec![r"C:\a.txt".to_string()]),
            ..Default::default()
        }
        .is_empty());
        assert!(!ClipboardPayload {
            image_png: Some(vec![1, 2, 3]),
            ..Default::default()
        }
        .is_empty());
        assert!(!ClipboardPayload {
            html: Some("<b>x</b>".to_string()),
            ..Default::default()
        }
        .is_empty());
        assert!(!ClipboardPayload {
            rtf: Some(b"{\\rtf1}".to_vec()),
            ..Default::default()
        }
        .is_empty());
    }
}
