//! One-time move of an existing install to the names this app uses now.
//!
//! The app used to be called PastePaw, so an install that has been used before keeps its
//! database and its settings under paths the renamed build would never look at. This
//! module moves them across once.
//!
//! It is deliberately careful with the user's data:
//! - a file that already exists in the new location is never overwritten,
//! - a rename is preferred, which moves rather than copies,
//! - when a copy is unavoidable the copy is verified against the original first, and
//!   the original is left where it is (the legacy directory is set aside afterwards
//!   rather than deleted).

use crate::constants::DB_FILE_NAME;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Directory name the app used before it was renamed.
pub const LEGACY_DIR_NAME: &str = "PastePaw";
/// The settings directory is named after the bundle identifier, which also changed.
pub const LEGACY_IDENTIFIER_DIR_NAME: &str = "me.xueshi.pastepaw";
/// Database file name before the rename.
pub const LEGACY_DB_FILE_NAME: &str = "paste_paw.db";
/// Suffix the legacy directory gets once its contents have been moved away.
pub const MIGRATED_SUFFIX: &str = "migrated";
/// Sub-directory holding the images belonging to image clips.
const IMAGES_DIR_NAME: &str = "images";

/// SQLite keeps its write-ahead log and shared memory next to the database file, and
/// leaving them behind would hand the new name a database with a stale log beside it.
const SQLITE_SIDECAR_SUFFIXES: [&str; 2] = ["-wal", "-shm"];

/// What a migration pass did, so the caller can log it and the tests can assert it.
#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct MigrationReport {
    pub database_moved: bool,
    pub images_moved: bool,
}

impl MigrationReport {
    pub fn moved_anything(&self) -> bool {
        self.database_moved || self.images_moved
    }
}

/// What the startup migration did, held until the logger is running.
///
/// The migration runs before the logging plugin is built, so logging its result at that
/// moment would go nowhere. It is reported from `setup` instead, through [`log_report`].
static REPORT: OnceLock<MigrationReport> = OnceLock::new();

/// The directory the app used to keep its data in, if it is still there.
fn legacy_data_dir() -> Option<PathBuf> {
    let root = dirs::data_dir()?;
    let path = root.join(LEGACY_DIR_NAME);
    path.is_dir().then_some(path)
}

/// The directory the app used to keep `settings.json` in, if it is still there.
fn legacy_settings_dir() -> Option<PathBuf> {
    let root = dirs::data_dir()?;
    let path = root.join(LEGACY_IDENTIFIER_DIR_NAME);
    path.is_dir().then_some(path)
}

/// Moves the database and the image directory of a legacy install into `current_data_dir`.
///
/// `current_data_dir` is normally created by the caller *after* this runs, so that a
/// migration can move the whole legacy directory rather than merge into a fresh one.
pub fn migrate_data(current_data_dir: &Path) -> MigrationReport {
    if crate::data_dir_is_overridden() {
        return MigrationReport::default();
    }

    let Some(legacy) = legacy_data_dir() else {
        return MigrationReport::default();
    };

    let report = MigrationReport {
        database_moved: matches!(migrate_database(&legacy, current_data_dir), Ok(true)),
        images_moved: matches!(migrate_images(&legacy, current_data_dir), Ok(true)),
    };

    if report.moved_anything() {
        set_legacy_dir_aside(&legacy);
    }

    let _ = REPORT.set(report);
    report
}

/// Reports what the migration did, once the logger exists.
pub fn log_report() {
    match REPORT.get() {
        Some(report) if report.moved_anything() => log::info!(
            "Migrated data from the previous name: database={} images={}",
            report.database_moved,
            report.images_moved
        ),
        Some(_) => log::debug!("No data needed migrating"),
        None => {}
    }
}

/// Moves `settings.json` into the directory belonging to the new identifier.
pub fn migrate_settings(current_settings_dir: &Path) -> bool {
    if crate::data_dir_is_overridden() {
        return false;
    }

    let Some(legacy) = legacy_settings_dir() else {
        return false;
    };
    matches!(
        migrate_settings_file(&legacy, current_settings_dir),
        Ok(true)
    )
}

/// Moves the database, and its SQLite sidecar files, from `legacy_dir` to `current_dir`.
pub fn migrate_database(legacy_dir: &Path, current_dir: &Path) -> Result<bool, String> {
    let source = legacy_dir.join(LEGACY_DB_FILE_NAME);
    let target = current_dir.join(DB_FILE_NAME);
    let moved = move_entry(&source, &target)?;

    for suffix in SQLITE_SIDECAR_SUFFIXES {
        let source = legacy_dir.join(format!("{}{}", LEGACY_DB_FILE_NAME, suffix));
        let target = current_dir.join(format!("{}{}", DB_FILE_NAME, suffix));
        move_entry(&source, &target)?;
    }

    Ok(moved)
}

/// Moves the whole image directory across.
pub fn migrate_images(legacy_dir: &Path, current_dir: &Path) -> Result<bool, String> {
    let source = legacy_dir.join(IMAGES_DIR_NAME);
    let target = current_dir.join(IMAGES_DIR_NAME);
    move_entry(&source, &target)
}

/// Moves `settings.json` from the legacy identifier directory to the current one.
pub fn migrate_settings_file(
    legacy_settings_dir: &Path,
    current_settings_dir: &Path,
) -> Result<bool, String> {
    let source = legacy_settings_dir.join("settings.json");
    let target = current_settings_dir.join("settings.json");
    move_entry(&source, &target)
}

/// Moves one file or directory, never overwriting anything.
///
/// A rename is atomic and works for directories, so it is tried first. It fails when the
/// two paths sit on different volumes, and then the contents are copied and verified. The
/// source is not removed in that case: the caller sets the legacy directory aside, so the
/// bytes the copy came from are still on disk if anything about the copy turns out wrong.
fn move_entry(source: &Path, target: &Path) -> Result<bool, String> {
    if !source.exists() {
        return Ok(false);
    }
    if target.exists() {
        // Something is already there under the new name: the user has run this build
        // before, and its data wins.
        return Ok(false);
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create {}: {}", parent.display(), e))?;
    }

    if fs::rename(source, target).is_ok() {
        return Ok(true);
    }

    copy_recursively(source, target)?;
    verify_copy(source, target)?;
    Ok(true)
}

fn copy_recursively(source: &Path, target: &Path) -> Result<(), String> {
    if source.is_dir() {
        fs::create_dir_all(target).map_err(|e| format!("create {}: {}", target.display(), e))?;
        let entries =
            fs::read_dir(source).map_err(|e| format!("read {}: {}", source.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            copy_recursively(&entry.path(), &target.join(entry.file_name()))?;
        }
        return Ok(());
    }

    fs::copy(source, target)
        .map(|_| ())
        .map_err(|e| format!("copy {}: {}", source.display(), e))
}

/// Confirms a copy holds what the original held, by file count and total size.
fn verify_copy(source: &Path, target: &Path) -> Result<(), String> {
    let expected = measure(source)?;
    let actual = measure(target)?;
    if expected != actual {
        return Err(format!(
            "copy of {} does not match the original ({} files/{} bytes vs {} files/{} bytes)",
            source.display(),
            expected.0,
            expected.1,
            actual.0,
            actual.1
        ));
    }
    Ok(())
}

/// `(files, bytes)` for a file or a directory tree.
fn measure(path: &Path) -> Result<(u64, u64), String> {
    if path.is_dir() {
        let mut files = 0;
        let mut bytes = 0;
        let entries = fs::read_dir(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let (child_files, child_bytes) = measure(&entry.path())?;
            files += child_files;
            bytes += child_bytes;
        }
        return Ok((files, bytes));
    }

    let bytes = fs::metadata(path)
        .map_err(|e| format!("stat {}: {}", path.display(), e))?
        .len();
    Ok((1, bytes))
}

/// Renames the legacy directory out of the way so nothing reads it again.
///
/// A rename, not a delete: whatever it still holds stays on disk under a name that says
/// what happened to it.
fn set_legacy_dir_aside(legacy_dir: &Path) {
    let name = legacy_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| LEGACY_DIR_NAME.to_string());
    let aside = legacy_dir.with_file_name(format!("{}.{}", name, MIGRATED_SUFFIX));
    if aside.exists() {
        return;
    }
    let _ = fs::rename(legacy_dir, &aside);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A directory under the system temp directory, removed when the test ends.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "vigilclip-migration-{}-{}",
                tag,
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        let mut file = fs::File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write file");
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).expect("read file")
    }

    #[test]
    fn the_legacy_database_is_renamed_into_place() {
        let legacy = TempDir::new("api-db-legacy");
        let current = TempDir::new("api-db-current");
        write(&legacy.path().join(LEGACY_DB_FILE_NAME), "clips");

        assert!(migrate_database(legacy.path(), current.path()).expect("migrate"));

        assert_eq!(read(&current.path().join(DB_FILE_NAME)), "clips");
        assert!(!legacy.path().join(LEGACY_DB_FILE_NAME).exists());
    }

    #[test]
    fn the_sqlite_sidecar_files_come_along() {
        let legacy = TempDir::new("sidecar-legacy");
        let current = TempDir::new("sidecar-current");
        write(&legacy.path().join(LEGACY_DB_FILE_NAME), "clips");
        write(
            &legacy.path().join(format!("{}-wal", LEGACY_DB_FILE_NAME)),
            "log",
        );

        migrate_database(legacy.path(), current.path()).expect("migrate");

        assert_eq!(
            read(&current.path().join(format!("{}-wal", DB_FILE_NAME))),
            "log"
        );
    }

    #[test]
    fn an_existing_database_is_never_overwritten() {
        let legacy = TempDir::new("keep-legacy");
        let current = TempDir::new("keep-current");
        write(&legacy.path().join(LEGACY_DB_FILE_NAME), "old install");
        write(&current.path().join(DB_FILE_NAME), "this build's own");

        assert!(!migrate_database(legacy.path(), current.path()).expect("migrate"));

        assert_eq!(read(&current.path().join(DB_FILE_NAME)), "this build's own");
        assert_eq!(
            read(&legacy.path().join(LEGACY_DB_FILE_NAME)),
            "old install"
        );
    }

    #[test]
    fn the_image_directory_comes_along_with_its_contents() {
        let legacy = TempDir::new("images-legacy");
        let current = TempDir::new("images-current");
        write(&legacy.path().join("images").join("a.png"), "png");

        assert!(migrate_images(legacy.path(), current.path()).expect("migrate"));

        assert_eq!(read(&current.path().join("images").join("a.png")), "png");
    }

    #[test]
    fn a_legacy_settings_file_moves_to_the_new_directory() {
        let legacy = TempDir::new("settings-legacy");
        let current = TempDir::new("settings-current");
        write(&legacy.path().join("settings.json"), "{}");

        assert!(migrate_settings_file(legacy.path(), current.path()).expect("migrate"));

        assert_eq!(read(&current.path().join("settings.json")), "{}");
    }

    #[test]
    fn migrating_twice_moves_nothing_the_second_time() {
        let legacy = TempDir::new("twice-legacy");
        let current = TempDir::new("twice-current");
        write(&legacy.path().join(LEGACY_DB_FILE_NAME), "clips");
        write(&legacy.path().join("images").join("a.png"), "png");

        let first = MigrationReport {
            database_moved: migrate_database(legacy.path(), current.path()).expect("migrate"),
            images_moved: migrate_images(legacy.path(), current.path()).expect("migrate"),
        };
        let second = MigrationReport {
            database_moved: migrate_database(legacy.path(), current.path()).expect("migrate"),
            images_moved: migrate_images(legacy.path(), current.path()).expect("migrate"),
        };

        assert!(first.moved_anything());
        assert!(!second.moved_anything());
        assert_eq!(
            second,
            MigrationReport {
                database_moved: false,
                images_moved: false
            }
        );
    }

    #[test]
    fn nothing_to_migrate_is_not_an_error() {
        let legacy = TempDir::new("empty-legacy");
        let current = TempDir::new("empty-current");

        let report = MigrationReport {
            database_moved: migrate_database(legacy.path(), current.path()).expect("migrate"),
            images_moved: migrate_images(legacy.path(), current.path()).expect("migrate"),
        };

        assert!(!report.moved_anything());
    }

    #[test]
    fn a_copy_is_verified_before_the_source_is_considered_moved() {
        let source = TempDir::new("verify-source");
        let target = TempDir::new("verify-target");
        write(&source.path().join("a.txt"), "12345");

        // A target that cannot be created (its parent is a file) makes the copy path fail
        // rather than silently succeed with nothing copied.
        let blocked = target.path().join("a-file");
        write(&blocked, "x");

        assert!(copy_recursively(source.path(), &blocked.join("nested")).is_err());
        assert!(source.path().join("a.txt").exists());
    }

    #[test]
    fn a_measure_of_a_tree_counts_files_and_bytes() {
        let dir = TempDir::new("measure");
        write(&dir.path().join("a.txt"), "12345");
        write(&dir.path().join("nested").join("b.txt"), "123");

        assert_eq!(measure(dir.path()).expect("measure"), (2, 8));
        assert_eq!(measure(&dir.path().join("a.txt")).expect("measure"), (1, 5));
    }

    #[test]
    fn verify_rejects_a_copy_that_lost_a_file() {
        let source = TempDir::new("verify-lost-source");
        let target = TempDir::new("verify-lost-target");
        write(&source.path().join("a.txt"), "12345");
        write(&target.path().join("a.txt"), "12345");
        write(&source.path().join("b.txt"), "123");

        assert!(verify_copy(source.path(), target.path()).is_err());
    }
}
