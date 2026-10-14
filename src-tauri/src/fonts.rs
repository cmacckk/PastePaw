//! The font families this machine has installed.
//!
//! The typography settings offer a font list, and a fixed list can only ever be wrong:
//! the fonts a Windows machine has depend on what the user installed. Windows is asked
//! instead, and the answer is cached for the life of the process because enumerating
//! fonts takes a few hundred milliseconds and the list cannot change under a running app
//! in any way that matters.

use std::collections::BTreeSet;

/// Sorted family names, de-duplicated, as the settings window shows them.
pub fn installed_font_families() -> &'static Vec<String> {
    static FONTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| normalise(enumerate_families()))
}

/// Trims, drops blanks, drops the vertical variants and sorts what is left.
///
/// Windows reports names as they are stored, which can include padding, and also reports
/// an `@`-prefixed variant for each family that supports vertical writing. Those variants
/// are the same font rotated: offering them would double the list for no benefit.
fn normalise(names: Vec<String>) -> Vec<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();

    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.starts_with('@') {
            continue;
        }
        // Windows treats family names case-insensitively, so a second spelling of a name
        // already in the list is the same font.
        if seen.insert(trimmed.to_lowercase()) {
            out.push(trimmed.to_string());
        }
    }

    out.sort_by_key(|name| name.to_lowercase());
    out
}

#[cfg(windows)]
fn enumerate_families() -> Vec<String> {
    use windows::Win32::Foundation::LPARAM;
    use windows::Win32::Graphics::Gdi::{
        EnumFontFamiliesExW, GetDC, ReleaseDC, DEFAULT_CHARSET, LOGFONTW, TEXTMETRICW,
    };

    /// Called once per font. Returning a non-zero value asks for the next one.
    unsafe extern "system" fn collect(
        logfont: *const LOGFONTW,
        _metric: *const TEXTMETRICW,
        _font_type: u32,
        lparam: LPARAM,
    ) -> i32 {
        unsafe {
            if logfont.is_null() {
                return 1;
            }
            let names = &mut *(lparam.0 as *mut Vec<String>);
            let face = &(*logfont).lfFaceName;
            let end = face.iter().position(|&c| c == 0).unwrap_or(face.len());
            if end > 0 {
                names.push(String::from_utf16_lossy(&face[..end]));
            }
            1
        }
    }

    let mut names: Vec<String> = Vec::new();
    // The screen device context is enough: installed fonts are a system-wide list, and
    // asking a printer context for them is how you end up with printer-resident fonts.
    let hdc = unsafe { GetDC(None) };
    if hdc.is_invalid() {
        log::warn!("Fonts: could not get a device context, the font list will be empty");
        return names;
    }

    let logfont = LOGFONTW {
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };

    unsafe {
        let _ = EnumFontFamiliesExW(
            hdc,
            &logfont,
            Some(collect),
            LPARAM(&mut names as *mut Vec<String> as isize),
            0,
        );
        let _ = ReleaseDC(None, hdc);
    }

    names
}

/// Only Windows is supported, so this exists for the other targets' benefit.
#[cfg(not(windows))]
fn enumerate_families() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_trimmed_and_sorted_without_regard_to_case() {
        let fonts = normalise(vec![
            "consolas".into(),
            "  Arial  ".into(),
            "Segoe UI".into(),
        ]);

        assert_eq!(fonts, vec!["Arial", "consolas", "Segoe UI"]);
    }

    #[test]
    fn blank_names_and_the_vertical_variants_are_left_out() {
        let fonts = normalise(vec![
            "".into(),
            "   ".into(),
            "@Microsoft YaHei".into(),
            "Microsoft YaHei".into(),
        ]);

        assert_eq!(fonts, vec!["Microsoft YaHei"]);
    }

    #[test]
    fn the_same_family_is_only_listed_once() {
        let fonts = normalise(vec!["Arial".into(), "arial".into(), "ARIAL".into()]);

        assert_eq!(fonts, vec!["Arial"], "the first spelling is kept");
    }

    #[test]
    fn an_empty_answer_is_not_an_error() {
        assert!(normalise(Vec::new()).is_empty());
    }

    #[test]
    fn the_installed_font_list_is_cached_and_plausible() {
        let fonts = installed_font_families();

        // A Windows machine without fonts cannot render anything, so anything close to
        // empty means the enumeration failed rather than that the answer is real.
        #[cfg(windows)]
        eprintln!("installed font families: {}", fonts.len());
        #[cfg(windows)]
        assert!(
            fonts.len() > 10,
            "expected the installed fonts, got {} entries",
            fonts.len()
        );
        assert!(
            fonts.iter().all(|name| !name.trim().is_empty()),
            "the list must not contain blank names"
        );
        assert!(
            std::ptr::eq(fonts, installed_font_families()),
            "the second call must return the same cached list"
        );
    }
}
