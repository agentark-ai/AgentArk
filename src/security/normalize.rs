//! Unicode normalization for detection surfaces.
//!
//! Attackers bypass string-matching guards with visually identical but
//! codepoint-distinct characters (Cyrillic `і` for Latin `i`), invisible
//! characters (zero-width joiners), or bidirectional override tricks. The LLM
//! tokenizer sees through these; a naive regex/substring check does not.
//!
//! This module produces a canonical form used ONLY for detection and
//! classification. The original text is what gets stored, shown to the user,
//! and sent to the LLM. We never silently rewrite user content.

use unicode_normalization::UnicodeNormalization;

/// Produce a canonical form of `text` suitable for structural analysis.
///
/// Steps:
/// 1. NFKC normalization — collapses fullwidth, ligatures, and compatibility
///    forms to their canonical representation.
/// 2. Strip invisible characters: zero-width space/joiner/non-joiner, word
///    joiner, byte-order mark.
/// 3. Strip bidirectional override controls which can be used to visually
///    reorder characters without changing their logical order.
pub fn normalize_for_analysis(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.nfkc() {
        if is_invisible(ch) || is_bidi_control(ch) {
            continue;
        }
        out.push(ch);
    }
    out
}

#[inline]
fn is_invisible(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}' // ZERO WIDTH SPACE
            | '\u{200C}' // ZERO WIDTH NON-JOINER
            | '\u{200D}' // ZERO WIDTH JOINER
            | '\u{2060}' // WORD JOINER
            | '\u{FEFF}' // BYTE ORDER MARK / ZERO WIDTH NO-BREAK SPACE
    )
}

#[inline]
fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{202A}' // LEFT-TO-RIGHT EMBEDDING
            | '\u{202B}' // RIGHT-TO-LEFT EMBEDDING
            | '\u{202C}' // POP DIRECTIONAL FORMATTING
            | '\u{202D}' // LEFT-TO-RIGHT OVERRIDE
            | '\u{202E}' // RIGHT-TO-LEFT OVERRIDE
            | '\u{2066}' // LEFT-TO-RIGHT ISOLATE
            | '\u{2067}' // RIGHT-TO-LEFT ISOLATE
            | '\u{2068}' // FIRST STRONG ISOLATE
            | '\u{2069}' // POP DIRECTIONAL ISOLATE
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullwidth_collapses_to_ascii() {
        let input = "\u{FF49}\u{FF47}\u{FF4E}\u{FF4F}\u{FF52}\u{FF45}"; // fullwidth "ignore"
        assert_eq!(normalize_for_analysis(input), "ignore");
    }

    #[test]
    fn zero_width_chars_are_stripped() {
        let input = "ign\u{200C}ore";
        assert_eq!(normalize_for_analysis(input), "ignore");
    }

    #[test]
    fn bidi_overrides_are_stripped() {
        let input = "\u{202E}ignore\u{202C}";
        assert_eq!(normalize_for_analysis(input), "ignore");
    }

    #[test]
    fn preserves_plain_text() {
        let input = "Hello, world!";
        assert_eq!(normalize_for_analysis(input), "Hello, world!");
    }

    #[test]
    fn preserves_non_latin_prose() {
        let input = "Привет мир";
        assert_eq!(normalize_for_analysis(input), "Привет мир");
    }

    #[test]
    fn ligatures_decompose() {
        // NFKC decomposes U+FB03 into "ffi".
        let input = "e\u{FB03}cient";
        assert_eq!(normalize_for_analysis(input), "efficient");
    }

    #[test]
    fn empty_string() {
        assert_eq!(normalize_for_analysis(""), "");
    }

    #[test]
    fn combination_of_tricks() {
        let input = "\u{202E}\u{FF49}\u{FF47}\u{200B}\u{FF4E}\u{FF4F}\u{FF52}\u{FF45}";
        assert_eq!(normalize_for_analysis(input), "ignore");
    }
}
