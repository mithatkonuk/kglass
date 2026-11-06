//! Safe handling of text that originates from the cluster.
//!
//! Everything a cluster returns (names, labels, log lines, YAML) is untrusted.
//! [`SafeText`] can only be built through [`sanitize`], so terminal escape
//! sequences in cluster data can never reach the screen as control commands.

use std::fmt;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Text that has passed through [`sanitize`]. There is no other constructor.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct SafeText(String);

impl SafeText {
    pub fn new(raw: &str) -> Self {
        SafeText(sanitize(raw))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SafeText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for SafeText {
    fn from(raw: &str) -> Self {
        SafeText::new(raw)
    }
}

impl From<String> for SafeText {
    fn from(raw: String) -> Self {
        SafeText::new(&raw)
    }
}

/// Strip terminal escape sequences and control characters.
///
/// Tabs and newlines become a single space (all callers render single-line cells).
pub fn sanitize(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: parameters until a final byte in 0x40..=0x7E.
                Some('[') => {
                    for n in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&n) {
                            break;
                        }
                    }
                }
                // OSC, DCS, SOS, PM, APC: string terminated by BEL or ESC \.
                Some(']' | 'P' | 'X' | '^' | '_') => {
                    while let Some(n) = chars.next() {
                        if n == '\x07' {
                            break;
                        }
                        if n == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Any other two-character escape: drop both.
                _ => {}
            },
            '\t' | '\n' => out.push(' '),
            // Bidi overrides/isolates, zero-width and line/paragraph separators can
            // reorder or hide neighbouring text.
            '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{2066}'..='\u{2069}'
            | '\u{feff}' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// Cut `s` to at most `max_width` display columns, ending with `…` when cut.
pub fn truncate(s: &str, max_width: usize) -> String {
    if s.width() <= max_width {
        return s.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    let budget = max_width - 1;
    let mut used = 0;
    let mut out = String::new();
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

/// Shorten `sha256:<64 hex>` to `sha256:9f2c…e81a`; anything else is returned unchanged.
pub fn shorten_digest(digest: &str) -> String {
    match digest.strip_prefix("sha256:") {
        Some(hex) if hex.len() > 16 && hex.chars().all(|c| c.is_ascii_hexdigit()) => {
            format!("sha256:{}…{}", &hex[..4], &hex[hex.len() - 4..])
        }
        _ => digest.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_and_osc_sequences() {
        assert_eq!(sanitize("\x1b[31mred\x1b[0m"), "red");
        assert_eq!(sanitize("a\x1b]0;evil title\x07b"), "ab");
        assert_eq!(
            sanitize("a\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\b"),
            "alinkb"
        );
    }

    #[test]
    fn replaces_control_chars() {
        assert_eq!(sanitize("a\tb"), "a b");
        assert_eq!(sanitize("a\rb\x07c\x7fd"), "abcd");
        assert_eq!(sanitize("a\nb"), "a b");
        assert_eq!(sanitize("a\u{9b}31mb"), "a31mb");
    }

    #[test]
    fn strips_bidi_and_zero_width_format_chars() {
        assert_eq!(sanitize("foo\u{202e}gnp.exe"), "foognp.exe");
        assert_eq!(
            sanitize("a\u{200b}b\u{2066}c\u{2069}d\u{feff}e\u{2028}f"),
            "abcdef"
        );
    }

    #[test]
    fn safetext_only_via_sanitize() {
        assert_eq!(SafeText::from("x\x1b[2Jy").as_str(), "xy");
    }

    #[test]
    fn truncate_by_display_width() {
        assert_eq!(truncate("api-7c9d8f-x2k4", 20), "api-7c9d8f-x2k4");
        assert_eq!(truncate("api-7c9d8f-x2k4", 8), "api-7c9…");
        assert_eq!(truncate("日本語ポッド", 7), "日本語…");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(truncate("abc", 1), "…");
    }

    #[test]
    fn shortens_sha256_digest() {
        let d = "sha256:9f2c4b7e1d0a3c5f6e7d8c9b0a1f2e3d4c5b6a7980f1e2d3c4b5a69788e81a";
        assert_eq!(shorten_digest(d), "sha256:9f2c…e81a");
        assert_eq!(shorten_digest("v2.14.0"), "v2.14.0");
    }
}
