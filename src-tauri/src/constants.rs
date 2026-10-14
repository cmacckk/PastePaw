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

/// Environment variable that points the app at a different data directory.
///
/// A development or test run sets it so that experimenting never reads or moves the
/// files belonging to an installed copy. `crate::migration` refuses to run while it is
/// set, for the same reason.
pub const DATA_DIR_ENV: &str = "VIGILCLIP_DATA_DIR";
