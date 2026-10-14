// keep sync with frontend (constants.ts)
pub const WINDOW_HEIGHT: f64 = 266.0;
pub const WINDOW_MARGIN: f64 = 8.0;

/// This app's own directory underneath the user's data directory.
///
/// `crate::migration` also uses it, together with the name the app used before, to find
/// data written by an older install.
pub const APP_DIR_NAME: &str = "VigilClip";

/// Name of the SQLite file inside [`APP_DIR_NAME`].
pub const DB_FILE_NAME: &str = "vigilclip.db";
