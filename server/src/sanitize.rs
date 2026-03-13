//! Input sanitization helpers for user-supplied strings.
//!
//! These functions protect against naughty-string vectors:
//! - Control characters (NUL, BEL, ESC, etc.)
//! - Unicode bi-directional override attacks (RTL override, LTR override)
//! - Zero-width characters used for homograph / spoofing attacks
//! - Excessively long strings that waste storage
//! - Path traversal components embedded in names
//!
//! Reference: <https://github.com/minimaxir/big-list-of-naughty-strings>

/// Unicode codepoints that are dangerous in user-facing text.
///
/// These are invisible or alter rendering layout in ways that can mislead users
/// (e.g., making "evil.exe" appear as "exe.live" via RTL override).
const DANGEROUS_CODEPOINTS: &[char] = &[
    '\u{0000}', // NUL
    '\u{0001}', // SOH
    '\u{0002}', // STX
    '\u{0003}', // ETX
    '\u{0004}', // EOT
    '\u{0005}', // ENQ
    '\u{0006}', // ACK
    '\u{0007}', // BEL  — can ring terminal bells, log injection
    '\u{0008}', // BS
    '\u{000B}', // VT
    '\u{000C}', // FF
    '\u{000E}', // SO
    '\u{000F}', // SI
    '\u{0010}', // DLE
    '\u{0011}', // DC1
    '\u{0012}', // DC2
    '\u{0013}', // DC3
    '\u{0014}', // DC4
    '\u{0015}', // NAK
    '\u{0016}', // SYN
    '\u{0017}', // ETB
    '\u{0018}', // CAN
    '\u{0019}', // EM
    '\u{001A}', // SUB
    '\u{001B}', // ESC  — ANSI escape sequences in logs / terminals
    '\u{001C}', // FS
    '\u{001D}', // GS
    '\u{001E}', // RS
    '\u{001F}', // US
    '\u{007F}', // DEL
    // C1 controls (0x80–0x9F): rarely used legitimately, often misinterpreted
    // by terminal emulators and web renderers. Can cause display corruption.
    '\u{0080}', '\u{0081}', '\u{0082}', '\u{0083}', '\u{0084}',
    '\u{0086}', '\u{0087}', '\u{0088}', '\u{0089}', '\u{008A}',
    '\u{008B}', '\u{008C}', '\u{008D}', '\u{008E}', '\u{008F}',
    '\u{0090}', '\u{0091}', '\u{0092}', '\u{0093}', '\u{0094}',
    '\u{0095}', '\u{0096}', '\u{0097}', '\u{0098}', '\u{0099}',
    '\u{009A}', '\u{009B}', '\u{009C}', '\u{009D}', '\u{009E}',
    '\u{009F}',
    // Bidi overrides — can reverse apparent text direction
    '\u{200E}', // LRM  — Left-to-Right Mark
    '\u{200F}', // RLM  — Right-to-Left Mark
    '\u{202A}', // LRE  — Left-to-Right Embedding
    '\u{202B}', // RLE  — Right-to-Left Embedding
    '\u{202C}', // PDF  — Pop Directional Formatting
    '\u{202D}', // LRO  — Left-to-Right Override
    '\u{202E}', // RLO  — Right-to-Left Override  ← classic filename spoofing
    '\u{2066}', // LRI  — Left-to-Right Isolate
    '\u{2067}', // RLI  — Right-to-Left Isolate
    '\u{2068}', // FSI  — First Strong Isolate
    '\u{2069}', // PDI  — Pop Directional Isolate
    // Zero-width characters — invisible but can alter string equality / length
    '\u{200B}', // Zero Width Space
    '\u{200C}', // Zero Width Non-Joiner
    '\u{200D}', // Zero Width Joiner
    '\u{FEFF}', // BOM / Zero Width No-Break Space
    '\u{FFFE}', // Non-character codepoint (sometimes appears as reversed BOM)
    // Interlinear annotation anchors — abused for text injection
    '\u{FFF9}', '\u{FFFA}', '\u{FFFB}',
    // Object replacement / replacement character signals
    '\u{FFFC}', // Object Replacement Character
];

/// Strip dangerous / invisible Unicode codepoints and trim whitespace.
///
/// Preserves normal printable Unicode (emoji, CJK, Arabic, etc.) but removes
/// control characters and bidi overrides that could be used for spoofing.
pub fn sanitize_text(input: &str) -> String {
    input
        .chars()
        .filter(|c| !DANGEROUS_CODEPOINTS.contains(c))
        .collect::<String>()
        .trim()
        .to_string()
}

/// Sanitize a user-facing display name (album name, gallery name, tag, server name, etc.)
///
/// 1. Strips dangerous codepoints via [`sanitize_text`].
/// 2. Collapses consecutive whitespace into a single space.
/// 3. Truncates to `max_len` **characters** (not bytes).
///
/// Returns `Err(reason)` if the result is empty after sanitization.
/// Callers wrap this in `AppError::BadRequest`.
pub fn sanitize_display_name(input: &str, max_len: usize) -> Result<String, &'static str> {
    let cleaned = sanitize_text(input);

    // Collapse runs of whitespace to single space
    let collapsed: String = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if collapsed.is_empty() {
        return Err("Name must not be empty");
    }

    // Truncate at character boundary
    let truncated: String = collapsed.chars().take(max_len).collect();
    Ok(truncated)
}

/// Sanitize a filename: strip path separators, traversal sequences,
/// control characters, and bidi overrides.
///
/// Falls back to a UUID-based name if the result is empty.
pub fn sanitize_filename(input: &str) -> String {
    let cleaned = sanitize_text(input);
    // Remove path separators and traversal
    let safe: String = cleaned
        .replace(['/', '\\'], "")
        .replace("..", "");

    let safe = safe.trim().to_string();

    if safe.is_empty() || safe == "." {
        format!("{}.bin", uuid::Uuid::new_v4())
    } else {
        // Truncate absurdly long filenames (filesystem limit is typically 255)
        safe.chars().take(255).collect()
    }
}

/// Validate that a file path is safe to join with a storage root:
/// - Must not contain `..`
/// - Must be relative (not start with `/` or a Windows drive letter)
/// - Must not contain NUL bytes
/// - Must not contain dangerous control characters
pub fn validate_relative_path(path: &str) -> Result<(), &'static str> {
    if path.contains("..") {
        return Err("Path must not contain '..'");
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err("Path must be relative, not absolute");
    }
    // Windows drive letters: C:\, D:\, etc.
    // Only checks single-byte ASCII drive letters (A-Z), which covers all
    // real Windows paths.
    if path.len() >= 2 && path.as_bytes()[1] == b':' {
        return Err("Path must not contain a drive letter");
    }
    if path.contains('\0') {
        return Err("Path must not contain NUL bytes");
    }
    // Strip dangerous codepoints and check it still matches
    let sanitized = sanitize_text(path);
    if sanitized != path.trim() {
        return Err("Path contains invalid control characters");
    }
    Ok(())
}

/// Escape SQL LIKE wildcard characters so a literal search term matches exactly.
///
/// `%` → `\%`, `_` → `\_`
///
/// The caller must use `ESCAPE '\'` in the LIKE clause.
pub fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Sanitize a free-form text field (descriptions, error messages, JSON blobs, etc.)
/// with a byte-size budget. Strips control chars and truncates.
pub fn sanitize_freeform(input: &str, max_bytes: usize) -> String {
    let cleaned = sanitize_text(input);
    if cleaned.len() <= max_bytes {
        return cleaned;
    }
    // Truncate at char boundary within max_bytes
    let mut end = max_bytes;
    while end > 0 && !cleaned.is_char_boundary(end) {
        end -= 1;
    }
    cleaned[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_text_strips_control_chars() {
        assert_eq!(sanitize_text("hello\x00world"), "helloworld");
        assert_eq!(sanitize_text("hello\x07world"), "helloworld");
        assert_eq!(sanitize_text("hello\x1bworld"), "helloworld");
    }

    #[test]
    fn test_sanitize_text_strips_bidi_overrides() {
        // RTL override attack: "photo\u{202E}gpj.exe" should become "photogpj.exe"
        assert_eq!(sanitize_text("photo\u{202E}gpj.exe"), "photogpj.exe");
    }

    #[test]
    fn test_sanitize_text_strips_zero_width_chars() {
        assert_eq!(sanitize_text("hello\u{200B}world"), "helloworld");
        assert_eq!(sanitize_text("\u{FEFF}hello"), "hello");
    }

    #[test]
    fn test_sanitize_text_preserves_normal_unicode() {
        assert_eq!(sanitize_text("日本語テスト"), "日本語テスト");
        assert_eq!(sanitize_text("مرحبا"), "مرحبا");
        assert_eq!(sanitize_text("🎉🎊🎈"), "🎉🎊🎈");
        assert_eq!(sanitize_text("café résumé"), "café résumé");
    }

    #[test]
    fn test_sanitize_display_name() {
        assert_eq!(sanitize_display_name("  My Album  ", 100).unwrap(), "My Album");
        assert_eq!(sanitize_display_name("My   Album", 100).unwrap(), "My Album");
        assert!(sanitize_display_name("", 100).is_err());
        assert!(sanitize_display_name("   ", 100).is_err());
        assert!(sanitize_display_name("\x00\x01\x02", 100).is_err());
        // Truncation
        assert_eq!(sanitize_display_name("abcdef", 3).unwrap(), "abc");
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("../../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_filename("photo\x00.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("/etc/shadow"), "etcshadow");
        // Empty input → UUID fallback (non-empty, ends with ".bin")
        let empty_result = sanitize_filename("");
        assert!(!empty_result.is_empty());
        assert!(empty_result.ends_with(".bin"));
    }

    #[test]
    fn test_validate_relative_path() {
        assert!(validate_relative_path("uploads/photo.jpg").is_ok());
        assert!(validate_relative_path("foo/bar/baz.png").is_ok());
        assert!(validate_relative_path("../../../etc/passwd").is_err());
        assert!(validate_relative_path("/etc/passwd").is_err());
        assert!(validate_relative_path("C:\\Windows\\System32").is_err());
        assert!(validate_relative_path("ok\x00bad").is_err());
    }

    #[test]
    fn test_escape_like() {
        assert_eq!(escape_like("hello"), "hello");
        assert_eq!(escape_like("100%"), "100\\%");
        assert_eq!(escape_like("a_b"), "a\\_b");
        assert_eq!(escape_like("a\\b"), "a\\\\b");
    }

    #[test]
    fn test_sanitize_freeform_truncation() {
        let long = "a".repeat(10000);
        let result = sanitize_freeform(&long, 4096);
        assert_eq!(result.len(), 4096);
    }

    #[test]
    fn test_naughty_strings_album_names() {
        // Script injection — should pass through (React escapes on render)
        // but control chars should be stripped
        let result = sanitize_display_name("<script>alert(1)</script>", 200);
        assert_eq!(result.unwrap(), "<script>alert(1)</script>");

        // SQL injection — should pass through (parameterized queries)
        let result = sanitize_display_name("'; DROP TABLE photos; --", 200);
        assert_eq!(result.unwrap(), "'; DROP TABLE photos; --");

        // Null byte injection
        let result = sanitize_display_name("album\x00name", 200);
        assert_eq!(result.unwrap(), "albumname");

        // RTL override
        let result = sanitize_display_name("album\u{202E}gpj.exe", 200);
        assert_eq!(result.unwrap(), "albumgpj.exe");

        // Emoji-heavy name
        let result = sanitize_display_name("🎉🎊🎈 Party Album 🥳", 200);
        assert_eq!(result.unwrap(), "🎉🎊🎈 Party Album 🥳");

        // Zalgo text — should pass (it's valid Unicode, just visually wild)
        let result = sanitize_display_name("Ṫ̈̃o̗̟͐ test", 200);
        assert!(result.is_ok());
    }
}
