//! Windows clipboard format encoding/decoding for the multi-format clipboard.
//!
//! This module is deliberately free of Win32 calls so the byte layouts can be
//! unit tested. The capture and write-back paths in `clipboard.rs` call into it.
//!
//! Formats handled:
//! - `CF_HDROP`  -> a `DROPFILES` header followed by a double-NUL terminated file list
//! - `CF_HTML`   -> an ASCII `Version:0.9` header carrying byte offsets, then HTML
//! - `CF_RTF`    -> opaque bytes, no header
//! - `CF_UNICODETEXT` -> plain text, handled elsewhere
#![allow(dead_code)] // TODO(v1.6 steps 2-4): remove once capture/write-back is wired in

/// Stored in `clip_formats.format`. Kept in sync with the `ClipType` union in
/// `frontend/src/types/index.ts`.
pub const FORMAT_TEXT: &str = "text";
pub const FORMAT_HTML: &str = "html";
pub const FORMAT_RTF: &str = "rtf";
pub const FORMAT_FILE: &str = "file";

// ---------------------------------------------------------------------------
// CF_HDROP / DROPFILES
// ---------------------------------------------------------------------------

/// Size of the Win32 `DROPFILES` header in bytes:
/// `DWORD pFiles` (4) + `POINT pt` (8) + `BOOL fNC` (4) + `BOOL fWide` (4).
pub const DROPFILES_HEADER_SIZE: usize = 20;

#[derive(Debug, PartialEq, Eq)]
pub enum HdropError {
    TooShort,
    BadOffset(usize),
}

/// Parses a `CF_HDROP` payload into the file paths it carries.
///
/// `pFiles` is the byte offset of the file list inside the payload and is not
/// always 20, so it must be read rather than assumed. `fWide` selects UTF-16LE
/// (`TRUE`, what every current Windows app sets) or ANSI.
///
/// ANSI entries are decoded as lossy UTF-8. That is correct for ASCII paths and
/// for UTF-8 capable locales, but not for legacy code pages such as GBK. The
/// capture path only accepts wide entries for that reason.
pub fn parse_hdrop(data: &[u8]) -> Result<Vec<String>, HdropError> {
    if data.len() < DROPFILES_HEADER_SIZE {
        return Err(HdropError::TooShort);
    }

    let p_files = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
    let f_wide = i32::from_le_bytes(data[16..20].try_into().unwrap()) != 0;

    if p_files < DROPFILES_HEADER_SIZE || p_files > data.len() {
        return Err(HdropError::BadOffset(p_files));
    }

    let list = &data[p_files..];
    Ok(if f_wide {
        decode_wide_list(list)
    } else {
        decode_ansi_list(list)
    })
}

/// Builds a `CF_HDROP` payload for the given paths, always as UTF-16LE.
pub fn build_hdrop(paths: &[String]) -> Vec<u8> {
    let mut out = vec![0u8; DROPFILES_HEADER_SIZE];
    out[0..4].copy_from_slice(&(DROPFILES_HEADER_SIZE as u32).to_le_bytes());
    // pt stays (0, 0) and fNC stays FALSE; only fWide must be set.
    out[16..20].copy_from_slice(&1i32.to_le_bytes());

    for path in paths {
        for unit in path.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    }
    // The list ends with an extra NUL, so an empty list is still one NUL.
    out.extend_from_slice(&0u16.to_le_bytes());

    out
}

fn decode_wide_list(list: &[u8]) -> Vec<String> {
    let (chunks, _trailing_odd_byte) = list.as_chunks::<2>();
    let mut paths = Vec::new();
    let mut current: Vec<u16> = Vec::new();

    for chunk in chunks {
        let unit = u16::from_le_bytes(*chunk);
        if unit == 0 {
            if current.is_empty() {
                break; // empty entry == end of list
            }
            paths.push(String::from_utf16_lossy(&current));
            current.clear();
        } else {
            current.push(unit);
        }
    }

    paths
}

fn decode_ansi_list(list: &[u8]) -> Vec<String> {
    let mut paths = Vec::new();
    for entry in list.split(|&b| b == 0) {
        if entry.is_empty() {
            break;
        }
        paths.push(String::from_utf8_lossy(entry).into_owned());
    }
    paths
}

// ---------------------------------------------------------------------------
// CF_DIB / CF_DIBV5
// ---------------------------------------------------------------------------

/// `sizeof(BITMAPINFOHEADER)`
pub const BITMAPINFOHEADER_SIZE: usize = 40;
/// `sizeof(BITMAPV5HEADER)`
pub const BITMAPV5HEADER_SIZE: usize = 124;

/// `BI_RGB`
const BI_RGB: u32 = 0;
/// `BI_BITFIELDS`
const BI_BITFIELDS: u32 = 3;
/// `LCS_sRGB`, i.e. the bytes `sRGB` read as a little-endian `DWORD`.
const LCS_SRGB: u32 = 0x7352_4742;
/// `LCS_GM_IMAGES`
const LCS_GM_IMAGES: u32 = 4;

#[derive(Debug, PartialEq, Eq)]
pub enum DibError {
    /// The image is empty and cannot be represented by a DIB.
    EmptyImage,
    /// `rgba` was not exactly `width * height * 4` bytes.
    WrongPixelBuffer { expected: usize, actual: usize },
}

/// Builds a `CF_DIB` payload: a `BITMAPINFOHEADER` followed by 32bpp `BI_RGB`
/// pixels.
///
/// The 4th byte of each pixel is nominally unused under `BI_RGB`, but the alpha
/// channel is kept in place because that is what applications and other clipboard
/// managers put there. Use [`build_dibv5`] when alpha has to be declared.
pub fn build_dib(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, DibError> {
    let pixels = to_bottom_up_bgra(rgba, width, height)?;
    let mut out = Vec::with_capacity(BITMAPINFOHEADER_SIZE + pixels.len());

    push_u32(&mut out, BITMAPINFOHEADER_SIZE as u32); // biSize
    push_i32(&mut out, width as i32); // biWidth
    push_i32(&mut out, height as i32); // biHeight, positive => bottom-up
    push_u16(&mut out, 1); // biPlanes
    push_u16(&mut out, 32); // biBitCount
    push_u32(&mut out, BI_RGB); // biCompression
    push_u32(&mut out, pixels.len() as u32); // biSizeImage
    push_i32(&mut out, 0); // biXPelsPerMeter
    push_i32(&mut out, 0); // biYPelsPerMeter
    push_u32(&mut out, 0); // biClrUsed
    push_u32(&mut out, 0); // biClrImportant
    debug_assert_eq!(out.len(), BITMAPINFOHEADER_SIZE);

    out.extend_from_slice(&pixels);
    Ok(out)
}

/// Builds a `CF_DIBV5` payload: a `BITMAPV5HEADER` followed by 32bpp
/// `BI_BITFIELDS` pixels with an explicit alpha mask.
///
/// This is the variant that carries transparency properly, so it is the one a
/// target application should prefer.
pub fn build_dibv5(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, DibError> {
    let pixels = to_bottom_up_bgra(rgba, width, height)?;
    let mut out = Vec::with_capacity(BITMAPV5HEADER_SIZE + pixels.len());

    push_u32(&mut out, BITMAPV5HEADER_SIZE as u32); // bV5Size
    push_i32(&mut out, width as i32); // bV5Width
    push_i32(&mut out, height as i32); // bV5Height, positive => bottom-up
    push_u16(&mut out, 1); // bV5Planes
    push_u16(&mut out, 32); // bV5BitCount
    push_u32(&mut out, BI_BITFIELDS); // bV5Compression
    push_u32(&mut out, pixels.len() as u32); // bV5SizeImage
    push_i32(&mut out, 0); // bV5XPelsPerMeter
    push_i32(&mut out, 0); // bV5YPelsPerMeter
    push_u32(&mut out, 0); // bV5ClrUsed
    push_u32(&mut out, 0); // bV5ClrImportant
                           // Channel masks. Because a DIB stores bytes little-endian, 0x00FF0000 for red
                           // means the byte order is blue, green, red, alpha.
    push_u32(&mut out, 0x00FF_0000); // bV5RedMask
    push_u32(&mut out, 0x0000_FF00); // bV5GreenMask
    push_u32(&mut out, 0x0000_00FF); // bV5BlueMask
    push_u32(&mut out, 0xFF00_0000); // bV5AlphaMask
    push_u32(&mut out, LCS_SRGB); // bV5CSType
    out.extend_from_slice(&[0u8; 36]); // bV5Endpoints, unused
    push_u32(&mut out, 0); // bV5GammaRed
    push_u32(&mut out, 0); // bV5GammaGreen
    push_u32(&mut out, 0); // bV5GammaBlue
    push_u32(&mut out, LCS_GM_IMAGES); // bV5Intent
    push_u32(&mut out, 0); // bV5ProfileData
    push_u32(&mut out, 0); // bV5ProfileSize
    push_u32(&mut out, 0); // bV5Reserved
    debug_assert_eq!(out.len(), BITMAPV5HEADER_SIZE);

    out.extend_from_slice(&pixels);
    Ok(out)
}

/// Converts top-down RGBA rows into the bottom-up BGRA buffer a DIB expects.
///
/// Two things are easy to get wrong here and both produce a visibly broken image
/// rather than an error: the row order is reversed for a positive height, and the
/// red and blue channels are swapped.
fn to_bottom_up_bgra(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, DibError> {
    if width == 0 || height == 0 {
        return Err(DibError::EmptyImage);
    }

    let (width, height) = (width as usize, height as usize);
    let expected = width * height * 4;
    if rgba.len() != expected {
        return Err(DibError::WrongPixelBuffer {
            expected,
            actual: rgba.len(),
        });
    }

    let mut out = vec![0u8; expected];
    for y in 0..height {
        let source = &rgba[y * width * 4..(y + 1) * width * 4];
        let destination = &mut out[(height - 1 - y) * width * 4..(height - y) * width * 4];

        // Both slices are exactly `width * 4` long, so there is never a remainder.
        let (source_pixels, _) = source.as_chunks::<4>();
        let (destination_pixels, _) = destination.as_chunks_mut::<4>();
        // Row 0 of the DIB is the bottom row of the image.
        for (src, dst) in source_pixels.iter().zip(destination_pixels.iter_mut()) {
            dst[0] = src[2]; // blue
            dst[1] = src[1]; // green
            dst[2] = src[0]; // red
            dst[3] = src[3]; // alpha
        }
    }

    Ok(out)
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

// ---------------------------------------------------------------------------
// CF_HTML
// ---------------------------------------------------------------------------

pub const CF_HTML_START_MARKER: &str = "<!--StartFragment-->";
pub const CF_HTML_END_MARKER: &str = "<!--EndFragment-->";
const CF_HTML_BODY_OPEN: &str = "<html><body>\r\n";
const CF_HTML_BODY_CLOSE: &str = "\r\n</body></html>";

/// Header layout. The offsets are zero padded to a fixed 10 digits on purpose:
/// that keeps the header byte length independent of the offset values, so the
/// offsets can be computed in one pass without a second fix-up iteration.
///
/// ```text
/// Version:0.9\r\n
/// StartHTML:0000000105\r\n
/// EndHTML:0000000226\r\n
/// StartFragment:0000000141\r\n
/// EndFragment:0000000200\r\n
/// [SourceURL:<url>\r\n]
/// ```
const CF_HTML_HEADER_PREFIX: &str = "Version:0.9\r\nStartHTML:";

/// Zero padded to a fixed width so the header length cannot change with the
/// values written into it.
const CF_HTML_OFFSET_WIDTH: usize = 10;

/// Builds a `CF_HTML` payload whose offsets point at the wrapped fragment.
///
/// Word, Outlook and browsers read `StartFragment`/`EndFragment` to decide what
/// to paste, so an off-by-one here shows up as garbage or truncated text in the
/// target application.
pub fn build_cf_html(fragment: &str, source_url: Option<&str>) -> Vec<u8> {
    // Pass 1: build the header with placeholder offsets purely to learn its
    // length. The values do not matter, only the width.
    let probe = header_text(0, 0, 0, 0, source_url);
    let header_len = probe.len();

    let start_html = header_len;
    let start_fragment = start_html + CF_HTML_BODY_OPEN.len() + CF_HTML_START_MARKER.len();
    let end_fragment = start_fragment + fragment.len();
    let end_html = end_fragment + CF_HTML_END_MARKER.len() + CF_HTML_BODY_CLOSE.len();

    // Pass 2: same widths, real values, so the length is unchanged.
    let header = header_text(
        start_html,
        end_html,
        start_fragment,
        end_fragment,
        source_url,
    );
    debug_assert_eq!(header.len(), header_len);

    let mut out = Vec::with_capacity(end_html);
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(CF_HTML_BODY_OPEN.as_bytes());
    out.extend_from_slice(CF_HTML_START_MARKER.as_bytes());
    out.extend_from_slice(fragment.as_bytes());
    out.extend_from_slice(CF_HTML_END_MARKER.as_bytes());
    out.extend_from_slice(CF_HTML_BODY_CLOSE.as_bytes());
    out
}

fn header_text(
    start_html: usize,
    end_html: usize,
    start_fragment: usize,
    end_fragment: usize,
    source_url: Option<&str>,
) -> String {
    // `format!` needs a literal, so the template is spelled out here rather than
    // shared as a const. Keep this in sync with `CF_HTML_HEADER_PREFIX`.
    let mut header = format!(
        "Version:0.9\r\nStartHTML:{:0width$}\r\nEndHTML:{:0width$}\r\nStartFragment:{:0width$}\r\nEndFragment:{:0width$}\r\n",
        start_html,
        end_html,
        start_fragment,
        end_fragment,
        width = CF_HTML_OFFSET_WIDTH,
    );
    if let Some(url) = source_url {
        header.push_str(&format!("SourceURL:{url}\r\n"));
    }
    header
}

/// Extracts the fragment from a `CF_HTML` payload produced by another app.
///
/// Other implementations are not guaranteed to emit a fragment range, so this
/// falls back to the whole document and strips the markers if present.
pub fn parse_cf_html(data: &[u8]) -> Option<String> {
    // Offsets are absolute byte positions from the start of the payload, so a
    // leading UTF-8 BOM (which Chrome emits) must NOT be stripped: the producer
    // counted those three bytes when it wrote the offsets.
    let header_len = cf_html_header_len(data)?;
    let header = &data[..header_len];

    let fragment = find_offset(header, b"StartFragment:")
        .zip(find_offset(header, b"EndFragment:"))
        .filter(|(start, end)| start < end && *end <= data.len())
        .and_then(|(start, end)| std::str::from_utf8(&data[start..end]).ok());

    if let Some(fragment) = fragment {
        return Some(fragment.to_string());
    }

    // Fallback: take the whole HTML document and strip the fragment markers.
    let start = find_offset(header, b"StartHTML:")?;
    let end = find_offset(header, b"EndHTML:").unwrap_or(data.len());
    let end = end.clamp(start, data.len());
    let html = std::str::from_utf8(&data[start..end]).ok()?;
    Some(
        html.trim()
            .trim_start_matches(CF_HTML_START_MARKER)
            .trim_end_matches(CF_HTML_END_MARKER)
            .to_string(),
    )
}

/// Byte offset where the header ends and the HTML document begins.
///
/// The offset is absolute, so it counts a leading BOM. The final line is allowed
/// to be unterminated: an HTML document is not required to contain a CRLF, and
/// demanding one here would reject otherwise valid payloads.
fn cf_html_header_len(data: &[u8]) -> Option<usize> {
    let mut position = if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };

    while position < data.len() {
        let (line, next) = match find_subslice(&data[position..], b"\r\n") {
            Some(offset) => {
                let line_end = position + offset;
                (&data[position..line_end], line_end + 2)
            }
            None => (&data[position..], data.len()),
        };

        if !is_header_line(line) {
            return Some(position);
        }
        position = next;
    }
    None
}

/// A header line looks like `Key:value` with an alphabetic key.
fn is_header_line(line: &[u8]) -> bool {
    match line.iter().position(|&b| b == b':') {
        Some(colon) if colon > 0 => line[..colon].iter().all(|b| b.is_ascii_alphabetic()),
        _ => false,
    }
}

/// Reads the decimal value following `key` in the header.
fn find_offset(header: &[u8], key: &[u8]) -> Option<usize> {
    let position = find_subslice(header, key)? + key.len();
    let digits: Vec<u8> = header[position..]
        .iter()
        .copied()
        .take_while(u8::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return None;
    }
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A short, human readable label for a file list, used as the clip preview.
pub fn describe_files(paths: &[String]) -> String {
    match paths {
        [] => String::new(),
        [only] => file_name_of(only).to_string(),
        [first, rest @ ..] => format!("{} +{}", file_name_of(first), rest.len()),
    }
}

/// The final component of a path, tolerating both separators and a trailing one.
pub fn file_name_of(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['\\', '/']);
    trimmed.rsplit(['\\', '/']).next().unwrap_or(trimmed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<String> {
        vec![
            r"C:\Users\Mechrevo\报告.txt".to_string(),
            r"D:\a b\c.png".to_string(),
        ]
    }

    // -- DROPFILES ---------------------------------------------------------

    #[test]
    fn hdrop_round_trips_unicode_paths() {
        let original = paths();
        let parsed = parse_hdrop(&build_hdrop(&original)).expect("parse");
        assert_eq!(parsed, original);
    }

    #[test]
    fn hdrop_writes_wide_flag_and_header_size() {
        let data = build_hdrop(&[r"C:\x".to_string()]);
        assert_eq!(
            u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize,
            DROPFILES_HEADER_SIZE
        );
        assert_ne!(i32::from_le_bytes(data[16..20].try_into().unwrap()), 0);
    }

    #[test]
    fn hdrop_empty_list_is_just_the_header_plus_terminator() {
        let data = build_hdrop(&[]);
        assert_eq!(data.len(), DROPFILES_HEADER_SIZE + 2);
        assert_eq!(parse_hdrop(&data).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn hdrop_rejects_short_payload() {
        assert_eq!(parse_hdrop(&[0u8; 4]), Err(HdropError::TooShort));
    }

    #[test]
    fn hdrop_rejects_offset_past_end() {
        let mut data = build_hdrop(&[r"C:\x".to_string()]);
        data[0..4].copy_from_slice(&9999u32.to_le_bytes());
        assert_eq!(parse_hdrop(&data), Err(HdropError::BadOffset(9999)));
    }

    #[test]
    fn hdrop_rejects_offset_inside_the_header() {
        let mut data = build_hdrop(&[r"C:\x".to_string()]);
        data[0..4].copy_from_slice(&8u32.to_le_bytes());
        assert_eq!(parse_hdrop(&data), Err(HdropError::BadOffset(8)));
    }

    /// Real payloads do not always use pFiles == 20, so the offset must be obeyed.
    #[test]
    fn hdrop_honours_a_non_standard_pfiles_offset() {
        let mut data = vec![0u8; DROPFILES_HEADER_SIZE + 4]; // 4 bytes of padding
        data[0..4].copy_from_slice(&((DROPFILES_HEADER_SIZE + 4) as u32).to_le_bytes());
        data[16..20].copy_from_slice(&1i32.to_le_bytes());
        for unit in r"C:\x".encode_utf16() {
            data.extend_from_slice(&unit.to_le_bytes());
        }
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());

        assert_eq!(parse_hdrop(&data).unwrap(), vec![r"C:\x".to_string()]);
    }

    #[test]
    fn hdrop_decodes_ansi_when_f_wide_is_false() {
        let mut data = vec![0u8; DROPFILES_HEADER_SIZE];
        data[0..4].copy_from_slice(&(DROPFILES_HEADER_SIZE as u32).to_le_bytes());
        data.extend_from_slice(b"C:\\a.txt\0D:\\b.txt\0\0");

        assert_eq!(
            parse_hdrop(&data).unwrap(),
            vec!["C:\\a.txt".to_string(), "D:\\b.txt".to_string()]
        );
    }

    // -- CF_HTML -----------------------------------------------------------

    #[test]
    fn cf_html_offsets_point_at_the_real_substrings() {
        let fragment = "<b>Hello 世界</b>";
        let data = build_cf_html(fragment, None);

        let header_len = cf_html_header_len(&data).expect("header");
        let start_fragment = find_offset(&data[..header_len], b"StartFragment:").unwrap();
        let end_fragment = find_offset(&data[..header_len], b"EndFragment:").unwrap();
        let start_html = find_offset(&data[..header_len], b"StartHTML:").unwrap();
        let end_html = find_offset(&data[..header_len], b"EndHTML:").unwrap();

        assert_eq!(start_html, header_len);
        assert_eq!(&data[start_fragment..end_fragment], fragment.as_bytes());
        assert_eq!(end_html, data.len());
        assert_eq!(
            std::str::from_utf8(&data[start_html..]).unwrap().len() + start_html,
            data.len()
        );
    }

    #[test]
    fn cf_html_round_trips() {
        let fragment = "<p>line one</p>\r\n<p>line two</p>";
        let data = build_cf_html(fragment, None);
        assert_eq!(parse_cf_html(&data).as_deref(), Some(fragment));
    }

    /// A multi-byte fragment must shift the offsets by bytes, not characters.
    #[test]
    fn cf_html_counts_bytes_not_characters() {
        let data = build_cf_html("中文中文中文", None);
        let header_len = cf_html_header_len(&data).unwrap();
        let start = find_offset(&data[..header_len], b"StartFragment:").unwrap();
        let end = find_offset(&data[..header_len], b"EndFragment:").unwrap();
        assert_eq!(end - start, "中文中文中文".len());
        assert_eq!(end - start, 18);
    }

    #[test]
    fn cf_html_header_length_is_stable_with_source_url() {
        let with_url = build_cf_html("<i>x</i>", Some("https://example.com/a"));
        let header_len = cf_html_header_len(&with_url).unwrap();
        let header = std::str::from_utf8(&with_url[..header_len]).unwrap();
        assert!(header.contains("SourceURL:https://example.com/a"));
        assert_eq!(parse_cf_html(&with_url).as_deref(), Some("<i>x</i>"));
    }

    /// Chrome, Word and Outlook emit a UTF-8 BOM and count its three bytes in
    /// every offset, so the payload must be parsed without stripping it first.
    #[test]
    fn cf_html_counts_a_leading_bom_in_its_offsets() {
        let fragment = "<b>bom</b>";
        let probe = format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n",
            0, 0, 0, 0
        );

        let start_html = 3 + probe.len(); // BOM + header
        let start_fragment = start_html + CF_HTML_BODY_OPEN.len() + CF_HTML_START_MARKER.len();
        let end_fragment = start_fragment + fragment.len();
        let end_html = end_fragment + CF_HTML_END_MARKER.len() + CF_HTML_BODY_CLOSE.len();
        let header = format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n",
            start_html, end_html, start_fragment, end_fragment
        );
        assert_eq!(header.len(), probe.len(), "header width must be stable");

        let mut data = vec![0xEF, 0xBB, 0xBF];
        data.extend_from_slice(header.as_bytes());
        data.extend_from_slice(CF_HTML_BODY_OPEN.as_bytes());
        data.extend_from_slice(CF_HTML_START_MARKER.as_bytes());
        data.extend_from_slice(fragment.as_bytes());
        data.extend_from_slice(CF_HTML_END_MARKER.as_bytes());
        data.extend_from_slice(CF_HTML_BODY_CLOSE.as_bytes());

        assert_eq!(cf_html_header_len(&data), Some(start_html));
        assert_eq!(parse_cf_html(&data).as_deref(), Some(fragment));
    }

    /// Some producers write only `StartHTML`/`EndHTML`. The whole document is
    /// then used, with the fragment markers stripped off.
    #[test]
    fn cf_html_falls_back_to_the_whole_document() {
        let html = "<!--StartFragment--><p>only</p><!--EndFragment-->";
        let start_html = format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\n",
            0, 0
        )
        .len();
        let header = format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\n",
            start_html,
            start_html + html.len()
        );
        assert_eq!(header.len(), start_html);

        let mut data = header.into_bytes();
        data.extend_from_slice(html.as_bytes());

        assert_eq!(parse_cf_html(&data).as_deref(), Some("<p>only</p>"));
    }

    /// The document body is not required to contain a CRLF. `build_cf_html`
    /// always emits one, so the payload is assembled by hand here.
    #[test]
    fn cf_html_parses_a_single_line_document() {
        let fragment = "<span>one-liner</span>";
        let body = format!(
            "<html><body>{CF_HTML_START_MARKER}{fragment}{CF_HTML_END_MARKER}</body></html>"
        );
        assert!(
            !body.contains("\r\n"),
            "payload must have no CRLF in the body"
        );

        let probe = format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n",
            0, 0, 0, 0
        );
        let start_html = probe.len();
        let start_fragment = start_html + "<html><body>".len() + CF_HTML_START_MARKER.len();
        let end_fragment = start_fragment + fragment.len();
        let end_html = start_html + body.len();
        let header = format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n",
            start_html, end_html, start_fragment, end_fragment
        );

        let mut data = header.into_bytes();
        data.extend_from_slice(body.as_bytes());

        assert_eq!(parse_cf_html(&data).as_deref(), Some(fragment));
    }

    #[test]
    fn cf_html_rejects_a_payload_without_a_header() {
        assert_eq!(parse_cf_html(b"<html><body>no header</body></html>"), None);
    }

    #[test]
    fn header_line_detection_stops_at_the_document() {
        assert!(is_header_line(b"Version:0.9"));
        assert!(is_header_line(b"SourceURL:https://a/b?c=d"));
        assert!(!is_header_line(b"<html><body>"));
        assert!(!is_header_line(b":novalue"));
        assert!(!is_header_line(b"<!--StartFragment-->"));
    }

    // -- helpers -----------------------------------------------------------

    #[test]
    fn find_subslice_locates_and_reports_missing() {
        assert_eq!(find_subslice(b"abcdef", b"cd"), Some(2));
        assert_eq!(find_subslice(b"abcdef", b"zz"), None);
        assert_eq!(find_subslice(b"abc", b""), None);
    }

    #[test]
    fn file_name_of_handles_separators_and_trailing_slashes() {
        assert_eq!(file_name_of(r"C:\Users\a\report.txt"), "report.txt");
        assert_eq!(file_name_of("C:/Users/a/report.txt"), "report.txt");
        assert_eq!(file_name_of(r"C:\Users\a\"), "a");
        assert_eq!(file_name_of("report.txt"), "report.txt");
        assert_eq!(file_name_of(""), "");
    }

    #[test]
    fn describe_files_is_short_for_long_lists() {
        let one = vec![r"C:\a\report.txt".to_string()];
        assert_eq!(describe_files(&one), "report.txt");

        let three = vec![
            r"C:\a\report.txt".to_string(),
            r"C:\a\b.png".to_string(),
            r"C:\a\c.doc".to_string(),
        ];
        assert_eq!(describe_files(&three), "report.txt +2");

        assert_eq!(describe_files(&[]), "");
    }

    #[test]
    fn format_names_are_stable() {
        // These strings are persisted in the `clip_formats.format` column and
        // read back by the frontend, so renaming one is a breaking change.
        assert_eq!(
            [FORMAT_TEXT, FORMAT_HTML, FORMAT_RTF, FORMAT_FILE],
            ["text", "html", "rtf", "file"]
        );
    }

    // -- CF_DIB / CF_DIBV5 -------------------------------------------------

    /// 2x2: top row red then green, bottom row blue then white.
    fn two_by_two() -> Vec<u8> {
        vec![
            0xFF, 0x00, 0x00, 0xFF, // red
            0x00, 0xFF, 0x00, 0xFF, // green
            0x00, 0x00, 0xFF, 0xFF, // blue
            0xFF, 0xFF, 0xFF, 0xFF, // white
        ]
    }

    #[test]
    fn dib_rows_are_flipped_and_channels_swapped() {
        let dib = build_dib(&two_by_two(), 2, 2).unwrap();
        let pixels = &dib[BITMAPINFOHEADER_SIZE..];

        // The bottom row comes first and every pixel is stored BGRA.
        assert_eq!(&pixels[0..4], &[0xFF, 0x00, 0x00, 0xFF]); // blue
        assert_eq!(&pixels[4..8], &[0xFF, 0xFF, 0xFF, 0xFF]); // white
        assert_eq!(&pixels[8..12], &[0x00, 0x00, 0xFF, 0xFF]); // red
        assert_eq!(&pixels[12..16], &[0x00, 0xFF, 0x00, 0xFF]); // green
    }

    #[test]
    fn dib_header_declares_a_bottom_up_32bpp_image() {
        let dib = build_dib(&two_by_two(), 2, 2).unwrap();

        assert_eq!(u32::from_le_bytes(dib[0..4].try_into().unwrap()), 40);
        assert_eq!(i32::from_le_bytes(dib[4..8].try_into().unwrap()), 2); // width
        assert_eq!(i32::from_le_bytes(dib[8..12].try_into().unwrap()), 2); // +height
        assert_eq!(u16::from_le_bytes(dib[12..14].try_into().unwrap()), 1); // planes
        assert_eq!(u16::from_le_bytes(dib[14..16].try_into().unwrap()), 32); // bpp
        assert_eq!(u32::from_le_bytes(dib[16..20].try_into().unwrap()), BI_RGB);
        assert_eq!(u32::from_le_bytes(dib[20..24].try_into().unwrap()), 16); // image size
    }

    #[test]
    fn dibv5_header_is_124_bytes_and_declares_an_alpha_mask() {
        let dib = build_dibv5(&two_by_two(), 2, 2).unwrap();

        assert_eq!(u32::from_le_bytes(dib[0..4].try_into().unwrap()), 124);
        assert_eq!(
            u32::from_le_bytes(dib[16..20].try_into().unwrap()),
            BI_BITFIELDS
        );
        assert_eq!(
            u32::from_le_bytes(dib[40..44].try_into().unwrap()),
            0x00FF_0000
        );
        assert_eq!(
            u32::from_le_bytes(dib[44..48].try_into().unwrap()),
            0x0000_FF00
        );
        assert_eq!(
            u32::from_le_bytes(dib[48..52].try_into().unwrap()),
            0x0000_00FF
        );
        assert_eq!(
            u32::from_le_bytes(dib[52..56].try_into().unwrap()),
            0xFF00_0000
        );
        assert_eq!(
            u32::from_le_bytes(dib[56..60].try_into().unwrap()),
            LCS_SRGB
        );
        assert_eq!(dib.len(), BITMAPV5HEADER_SIZE + 16);
    }

    #[test]
    fn dib_and_dibv5_carry_identical_pixels() {
        let rgba = two_by_two();
        let dib = build_dib(&rgba, 2, 2).unwrap();
        let dibv5 = build_dibv5(&rgba, 2, 2).unwrap();

        assert_eq!(
            &dib[BITMAPINFOHEADER_SIZE..],
            &dibv5[BITMAPV5HEADER_SIZE..],
            "the two DIB variants must not disagree about the pixels"
        );
    }

    #[test]
    fn dib_keeps_the_alpha_channel() {
        let rgba = vec![0x10, 0x20, 0x30, 0x40];
        let dib = build_dib(&rgba, 1, 1).unwrap();
        assert_eq!(&dib[BITMAPINFOHEADER_SIZE..], &[0x30, 0x20, 0x10, 0x40]);
    }

    #[test]
    fn dib_rejects_a_pixel_buffer_of_the_wrong_length() {
        assert_eq!(
            build_dib(&[0u8; 8], 2, 2),
            Err(DibError::WrongPixelBuffer {
                expected: 16,
                actual: 8
            })
        );
    }

    #[test]
    fn dib_rejects_a_zero_sized_image() {
        assert_eq!(build_dib(&[], 0, 0), Err(DibError::EmptyImage));
        assert_eq!(build_dib(&[], 4, 0), Err(DibError::EmptyImage));
        assert_eq!(build_dibv5(&[], 0, 4), Err(DibError::EmptyImage));
    }
}
