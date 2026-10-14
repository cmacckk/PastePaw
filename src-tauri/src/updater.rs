//! Update checks against the GitHub releases of this fork.
//!
//! The Tauri updater plugin is deliberately not used. Its endpoint and public
//! key belonged to the upstream repository, so a release published there would
//! have been offered to - and accepted by - users of this fork, quietly
//! replacing the fork build with upstream's and bringing back the code this
//! fork removed.
//!
//! Nothing is downloaded or installed here. The application reads the newest
//! release of its own repository and, when it is newer than the running build,
//! reports it; the update button then opens the release page in the browser.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;

/// The release feed this build reads: the newest published, non-prerelease
/// release of the fork.
pub const RELEASES_API: &str = "https://api.github.com/repos/cmacckk/VigilClip/releases/latest";

/// Where the update button sends the user. The app never installs anything.
pub const RELEASES_PAGE: &str = "https://github.com/cmacckk/VigilClip/releases/latest";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(1);
const CHECK_INTERVAL: Duration = Duration::from_secs(4 * 3600);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub body: String,
}

/// The parts of a GitHub release payload this application reads. Everything is
/// optional except the tag, so a feed that grows new fields keeps working.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

fn strip_version_prefix(value: &str) -> &str {
    value
        .strip_prefix('v')
        .or_else(|| value.strip_prefix('V'))
        .unwrap_or(value)
}

/// Turns "v1.6.1", "1.6.1" or "1.6.1-beta.2" into numbers that can be compared.
/// A prerelease keeps the numbers of its base release, so 1.6.0-beta.1 is not
/// newer than 1.6.0: GitHub never serves prereleases from this endpoint anyway,
/// and offering a beta as an update would be a nag rather than a help.
fn version_key(value: &str) -> Option<Vec<u64>> {
    let trimmed = strip_version_prefix(value.trim());
    let core = trimmed.split(['-', '+']).next()?;
    let parts: Option<Vec<u64>> = core
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect();
    match parts {
        Some(numbers) if !numbers.is_empty() => Some(numbers),
        _ => None,
    }
}

/// True when `latest` names a later version than `current`.
///
/// A version this code cannot read is answered with `false`. Guessing would
/// mean telling users their build is out of date on the strength of a string
/// this application failed to understand.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (version_key(latest), version_key(current)) {
        (Some(latest_parts), Some(current_parts)) => latest_parts > current_parts,
        _ => false,
    }
}

/// Reads a release payload and decides whether it is worth telling the user
/// about. Split out from the network call so the decision itself is testable.
pub fn release_from_json(json: &str, current: &str) -> Result<Option<UpdateInfo>, String> {
    let release: GithubRelease =
        serde_json::from_str(json).map_err(|e| format!("Unreadable release feed: {e}"))?;

    if release.draft || release.prerelease {
        return Ok(None);
    }

    if !is_newer(&release.tag_name, current) {
        return Ok(None);
    }

    Ok(Some(UpdateInfo {
        version: strip_version_prefix(&release.tag_name).to_string(),
        body: release.body,
    }))
}

/// Asks GitHub for the newest release of this fork.
///
/// A network problem is an error so the settings window can say so when the
/// user asked for a check. The background loop ignores the outcome entirely.
pub async fn fetch_latest_release() -> Result<Option<UpdateInfo>, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("VigilClip/", env!("CARGO_PKG_VERSION")))
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("Could not build an HTTP client: {e}"))?;

    let response = client
        .get(RELEASES_API)
        .send()
        .await
        .map_err(|e| format!("Could not reach the release feed: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("The release feed answered {}", response.status()));
    }

    let body = response
        .text()
        .await
        .map_err(|e| format!("Unreadable release feed: {e}"))?;

    release_from_json(&body, env!("CARGO_PKG_VERSION"))
}

#[derive(Clone, Default)]
pub struct UpdateManager {
    available_version: Arc<RwLock<Option<UpdateInfo>>>,
    is_checking: Arc<AtomicBool>,
}

impl UpdateManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// The remembered answer, checking once if nothing is known yet.
    pub async fn get_available_version(&self, app: &AppHandle) -> Option<UpdateInfo> {
        let cached = self.available_version.read().await.clone();
        if cached.is_some() {
            return cached;
        }
        self.check_for_updates(app).await.ok().flatten()
    }

    /// A second caller arriving while a check is running gets the cached answer
    /// rather than starting a second request.
    pub async fn check_for_updates(&self, app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
        if self
            .is_checking
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            log::info!("[UpdateManager] A check is already running; answering from the cache");
            return Ok(self.available_version.read().await.clone());
        }

        let result = self.do_check(app).await;
        self.is_checking.store(false, Ordering::SeqCst);
        result
    }

    async fn do_check(&self, app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
        match fetch_latest_release().await {
            Ok(Some(info)) => {
                log::info!("Update available: v{}", info.version);
                *self.available_version.write().await = Some(info.clone());
                let _ = app.emit("update-available", Some(info.clone()));
                Ok(Some(info))
            }
            Ok(None) => {
                log::info!("On the newest release");
                *self.available_version.write().await = None;
                let _ = app.emit("update-available", None::<UpdateInfo>);
                Ok(None)
            }
            Err(e) => {
                log::warn!("Failed to check for updates: {e}");
                Err(e)
            }
        }
    }

    pub fn start_background_loop(&self, app: AppHandle) {
        let manager = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(FIRST_CHECK_DELAY).await;
            let _ = manager.check_for_updates(&app).await;

            let mut interval = tokio::time::interval(CHECK_INTERVAL);
            interval.tick().await; // consume the first, immediate tick
            loop {
                interval.tick().await;
                let _ = manager.check_for_updates(&app).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_json(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","body":"Notes for {tag}","draft":false,"prerelease":false}}"#
        )
    }

    #[test]
    fn a_later_version_is_newer() {
        assert!(is_newer("1.6.1", "1.6.0"), "a patch release is newer");
        assert!(is_newer("v1.7.0", "1.6.0"), "a v prefix must not hide it");
        assert!(is_newer("2.0", "1.6.0"), "a shorter version still compares");
        assert!(is_newer("1.10.0", "1.9.0"), "10 is a number, not a letter");
    }

    #[test]
    fn the_running_version_and_anything_older_are_not_newer() {
        assert!(
            !is_newer("1.6.0", "1.6.0"),
            "the running version is not an update"
        );
        assert!(!is_newer("v1.6.0", "1.6.0"), "the same, with a prefix");
        assert!(
            !is_newer("1.5.9", "1.6.0"),
            "an older release is not an update"
        );
    }

    #[test]
    fn a_prerelease_of_the_running_version_is_not_newer() {
        assert!(!is_newer("1.6.0-beta.2", "1.6.0"));
    }

    #[test]
    fn a_version_that_cannot_be_read_is_never_an_update() {
        assert!(!is_newer("nightly", "1.6.0"));
        assert!(!is_newer("1.7.0", "unknown"));
    }

    #[test]
    fn a_newer_release_becomes_an_update_offer() {
        let info = release_from_json(&release_json("v1.6.1"), "1.6.0")
            .expect("valid payload")
            .expect("1.6.1 is newer than 1.6.0");
        assert_eq!(
            info.version, "1.6.1",
            "the v prefix is stripped for display"
        );
        assert_eq!(
            info.body, "Notes for v1.6.1",
            "release notes are carried over"
        );
    }

    #[test]
    fn a_release_for_the_running_version_is_not_offered() {
        assert_eq!(
            release_from_json(&release_json("v1.6.0"), "1.6.0"),
            Ok(None)
        );
    }

    #[test]
    fn drafts_and_prereleases_are_not_offered() {
        let draft = r#"{"tag_name":"v1.7.0","body":"","draft":true,"prerelease":false}"#;
        let prerelease = r#"{"tag_name":"v1.7.0","body":"","draft":false,"prerelease":true}"#;
        assert_eq!(
            release_from_json(draft, "1.6.0"),
            Ok(None),
            "a draft is not published"
        );
        assert_eq!(
            release_from_json(prerelease, "1.6.0"),
            Ok(None),
            "a prerelease is offered to nobody"
        );
    }

    #[test]
    fn a_payload_without_the_optional_fields_still_works() {
        let minimal = r#"{"tag_name":"v1.7.0"}"#;
        let info = release_from_json(minimal, "1.6.0")
            .expect("valid payload")
            .expect("1.7.0 is newer than 1.6.0");
        assert_eq!(
            info.body, "",
            "missing notes become empty notes, not a failure"
        );
    }

    #[test]
    fn an_unreadable_feed_is_reported_rather_than_panicking() {
        let error = release_from_json("<html>rate limited</html>", "1.6.0")
            .expect_err("HTML is not a release payload");
        assert!(
            error.contains("Unreadable release feed"),
            "the error should say what went wrong, got: {error}"
        );
    }
}
