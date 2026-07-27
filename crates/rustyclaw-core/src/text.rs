//! Text helpers shared across crates.
//!
//! Rust's `&s[..n]` panics when `n` lands inside a multi-byte character, so
//! any "truncate for display" that indexes by a raw number is a crash waiting
//! for its first emoji, CJK, or accented input. These helpers are the safe
//! way to do it: prefer [`truncate_chars`] for display budgets, and
//! [`truncate_bytes`] when the budget is genuinely about wire/byte size.

use std::borrow::Cow;

/// Truncate `s` to at most `max_chars` **characters**, appending `…` when it
/// actually truncates.
///
/// Counts by character rather than byte, which is what callers almost always
/// mean by "cap this at 50". Never panics and never splits a character.
///
/// ```
/// # use rustyclaw_core::text::truncate_chars;
/// assert_eq!(truncate_chars("hello", 10), "hello");
/// assert_eq!(truncate_chars("hello world", 5), "hello…");
/// // Multi-byte input is measured in characters, not bytes.
/// assert_eq!(truncate_chars("日本語のテキスト", 3), "日本語…");
/// ```
pub fn truncate_chars(s: &str, max_chars: usize) -> Cow<'_, str> {
    // Cheap path: bytes are an upper bound on characters, so a string that
    // fits the budget in bytes always fits it in characters.
    if s.len() <= max_chars {
        return Cow::Borrowed(s);
    }
    match s.char_indices().nth(max_chars) {
        // `idx` comes from `char_indices`, so it is always a valid boundary.
        Some((idx, _)) => Cow::Owned(format!("{}…", &s[..idx])),
        None => Cow::Borrowed(s),
    }
}

/// Truncate `s` to at most `max_bytes` bytes, snapping **down** to the
/// nearest character boundary. No ellipsis is added — the caller decides how
/// to mark the truncation.
///
/// Use this only when the budget is really about size (a wire limit, a log
/// cap); use [`truncate_chars`] for anything user-facing.
///
/// ```
/// # use rustyclaw_core::text::truncate_bytes;
/// assert_eq!(truncate_bytes("hello", 10), "hello");
/// // Would split a 3-byte character, so it snaps back instead of panicking.
/// assert_eq!(truncate_bytes("日本語", 4), "日");
/// ```
pub fn truncate_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_leaves_short_input_alone() {
        assert_eq!(truncate_chars("", 5), "");
        assert_eq!(truncate_chars("abc", 5), "abc");
        // Exactly at the budget: no ellipsis.
        assert_eq!(truncate_chars("abcde", 5), "abcde");
    }

    #[test]
    fn truncate_chars_counts_characters_not_bytes() {
        // 8 characters, 24 bytes: a byte-based cap of 10 would have cut this
        // mid-character and panicked.
        let s = "日本語のテキスト";
        assert_eq!(s.len(), 24);
        assert_eq!(s.chars().count(), 8);
        assert_eq!(truncate_chars(s, 8), s, "fits exactly, unchanged");
        assert_eq!(truncate_chars(s, 3), "日本語…");
    }

    #[test]
    fn truncate_chars_handles_emoji_and_combining_marks() {
        assert_eq!(truncate_chars("🦞🦀🐙", 2), "🦞🦀…");
        // Accented Latin (2 bytes per char).
        assert_eq!(truncate_chars("ünïcödé", 3), "ünï…");
    }

    /// The exact input that panicked the gateway's thread labeller.
    #[test]
    fn truncate_chars_survives_the_regression_input() {
        let msg = "日本語のメッセージをここに書きます、これはとても長いテキストです";
        assert!(msg.len() > 50, "must exceed a 50-*byte* budget");
        let out = truncate_chars(msg, 50);
        assert_eq!(out, msg, "50 characters is more than this input has");
        // And a budget that does bite still does not panic.
        assert_eq!(truncate_chars(msg, 5), "日本語のメ…");
    }

    #[test]
    fn truncate_bytes_snaps_down_to_a_boundary() {
        assert_eq!(truncate_bytes("hello", 10), "hello");
        assert_eq!(truncate_bytes("hello", 3), "hel");
        // 3-byte characters: budgets of 4, 5 and 6 all land safely.
        assert_eq!(truncate_bytes("日本語", 4), "日");
        assert_eq!(truncate_bytes("日本語", 5), "日");
        assert_eq!(truncate_bytes("日本語", 6), "日本");
        // A budget smaller than the first character yields empty, not a panic.
        assert_eq!(truncate_bytes("日本語", 1), "");
        assert_eq!(truncate_bytes("日本語", 0), "");
    }
}
