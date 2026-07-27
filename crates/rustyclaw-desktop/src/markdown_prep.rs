//! Preparation of assistant markdown before `dioxus-genai-chat` renders it.
//!
//! The chat crate renders Markdown with pulldown-cmark straight into
//! `dangerous_inner_html`, so everything it emits lands in the webview DOM.
//! Two consequences drive this module:
//!
//! 1. **Sanitising the markdown *source* is not enough.** `ammonia` cleans
//!    raw HTML, but it runs before pulldown-cmark and therefore never sees the
//!    `<a>`/`<img>` that pulldown-cmark generates from *markdown* syntax. A
//!    `[click](javascript:…)` link sailed straight through into the DOM. Link
//!    and image destinations are checked here against a scheme allowlist.
//!
//! 2. **Bare URLs were not links at all.** The chat crate enables only tables
//!    and strikethrough, and CommonMark does not autolink bare URLs, so
//!    `https://example.com` rendered as plain text — and `**https://…**`, the
//!    shape models most often emit, rendered as bold plain text. Bare URLs in
//!    ordinary prose are turned into real links here.
//!
//! Both passes locate their targets with pulldown-cmark's offset iterator
//! rather than by pattern-matching the source, so code spans, fenced blocks,
//! and existing links are identified by the same parser that will do the
//! final render — no regex approximation of Markdown.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// URL schemes allowed to become links. Anything else — `javascript:`,
/// `data:`, `file:`, `vbscript:`, custom app schemes — is rendered as inert
/// text instead.
const SAFE_SCHEMES: &[&str] = &["http://", "https://", "mailto:"];

/// Schemes that get autolinked when they appear bare in prose. Narrower than
/// [`SAFE_SCHEMES`]: a bare `mailto:` is rare and reads fine as text.
const AUTOLINK_SCHEMES: &[&str] = &["https://", "http://"];

/// Parser options — must match what `dioxus-genai-chat` uses, so this pass
/// sees exactly the document the renderer will see.
fn parser_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options
}

/// Whether `url` may be used as a link or image destination.
///
/// Relative URLs and fragments are permitted — they cannot execute script.
/// Comparison is case-insensitive because `JavaScript:` is equally effective.
fn is_safe_url(url: &str) -> bool {
    let trimmed = url.trim();
    // Control characters are stripped by parsers and can be used to smuggle a
    // scheme past a naive prefix check ("java\nscript:").
    let normalized: String = trimmed
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();

    if normalized.is_empty() {
        return true;
    }
    if SAFE_SCHEMES.iter().any(|s| normalized.starts_with(s)) {
        return true;
    }
    // No scheme at all → relative path, query, or fragment. `//host/path` is
    // protocol-relative and resolves to the page scheme, which is safe here.
    let Some(scheme_end) = normalized.find(':') else {
        return true;
    };
    // A ':' after a '/', '?' or '#' is part of the path, not a scheme
    // (e.g. "path/to:file"). Otherwise a real scheme is present, and it
    // failed the allowlist check above.
    normalized[..scheme_end].contains(['/', '?', '#'])
}

/// An edit to apply to the source, as a byte range and its replacement.
struct Edit {
    range: Range<usize>,
    replacement: String,
}

/// Prepare assistant markdown for rendering: neutralise unsafe link and image
/// destinations, then autolink bare URLs in prose.
///
/// Raw-HTML sanitisation (`ammonia`) is the caller's job and must happen
/// first; this pass assumes it is looking at already-cleaned source.
pub fn prepare(src: &str) -> String {
    let mut edits: Vec<Edit> = Vec::new();

    // Spans already claimed by an outer edit; inner edits inside them are
    // dropped so replacements can never overlap.
    let mut blocked: Vec<Range<usize>> = Vec::new();
    // Depth of enclosing links: text inside a link must not be autolinked, or
    // we would nest an <a> inside an <a>.
    let mut link_depth = 0usize;

    // pulldown-cmark splits text at delimiter runs, so a single URL can span
    // several `Text` events — `…/Rust_(language)` arrives as three. Scanning
    // each event separately would truncate the URL at the underscore, so
    // source-contiguous runs are coalesced and scanned as one.
    let mut run: Option<Range<usize>> = None;

    for (event, range) in Parser::new_ext(src, parser_options()).into_offset_iter() {
        // Any event other than a continuing text run terminates the run.
        let continues_run = matches!(event, Event::Text(_))
            && link_depth == 0
            && run.as_ref().is_some_and(|r| r.end == range.start);

        if !continues_run && let Some(finished) = run.take() {
            collect_autolinks(src, &finished, &mut edits);
        }

        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                if !is_safe_url(&dest_url) {
                    edits.push(Edit {
                        replacement: inert_text(src, &range),
                        range: range.clone(),
                    });
                    blocked.push(range);
                }
                link_depth += 1;
            }
            Event::End(TagEnd::Link) => {
                link_depth = link_depth.saturating_sub(1);
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                if !is_safe_url(&dest_url) {
                    edits.push(Edit {
                        replacement: inert_text(src, &range),
                        range: range.clone(),
                    });
                    blocked.push(range);
                }
                // Alt text counts as "inside a link" for autolinking purposes.
                // Injecting `[url](<url>)` into an `![…]` span rewrites the
                // document for no benefit — alt text cannot hold a link — and
                // relies on CommonMark flattening it back out to render
                // correctly, which extra brackets in the alt text can defeat.
                link_depth += 1;
            }
            Event::End(TagEnd::Image) => {
                link_depth = link_depth.saturating_sub(1);
            }
            // Only plain text outside links is a candidate for autolinking.
            // Code spans and fenced blocks arrive as `Event::Code` /
            // `Event::Text` inside `Tag::CodeBlock`, which are excluded here,
            // so URLs in code stay verbatim.
            Event::Text(_) if link_depth == 0 => {
                let inside_blocked = blocked
                    .iter()
                    .any(|b| b.start <= range.start && range.end <= b.end);
                if !inside_blocked {
                    run = Some(match run {
                        Some(r) => r.start..range.end,
                        None => range,
                    });
                }
            }
            _ => {}
        }
    }
    if let Some(finished) = run {
        collect_autolinks(src, &finished, &mut edits);
    }

    apply_edits(src, edits)
}

/// The rendered text of a span, with markdown link syntax defanged so the
/// blocked destination cannot be re-parsed into a link.
fn inert_text(src: &str, range: &Range<usize>) -> String {
    src[range.clone()]
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// Find bare URLs inside one text span and queue link replacements.
fn collect_autolinks(src: &str, range: &Range<usize>, edits: &mut Vec<Edit>) {
    let text = &src[range.clone()];
    let mut search_from = 0usize;

    while search_from < text.len() {
        let Some((rel_start, scheme)) = AUTOLINK_SCHEMES
            .iter()
            .filter_map(|s| text[search_from..].find(s).map(|i| (search_from + i, *s)))
            .min_by_key(|(i, _)| *i)
        else {
            break;
        };

        // Require a boundary before the scheme so "xhttps://…" is left alone.
        let preceded_ok = rel_start == 0
            || text[..rel_start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || "([<\"'".contains(c));
        if !preceded_ok {
            search_from = rel_start + scheme.len();
            continue;
        }

        let url_end = url_extent(text, rel_start);
        let url = &text[rel_start..url_end];
        // A scheme with nothing after it is not a URL.
        if url.len() > scheme.len() {
            edits.push(Edit {
                range: (range.start + rel_start)..(range.start + url_end),
                // Angle-bracket destination so URLs containing parentheses
                // survive; the visible text stays the raw URL.
                replacement: format!("[{}](<{}>)", url, url.replace(['<', '>'], "")),
            });
        }
        search_from = url_end.max(rel_start + scheme.len());
    }
}

/// Extent of the URL starting at `start`, trimming trailing punctuation that
/// is almost always sentence structure rather than part of the link.
fn url_extent(text: &str, start: usize) -> usize {
    let mut end = start;
    for (offset, ch) in text[start..].char_indices() {
        if ch.is_whitespace() || ch == '<' || ch == '>' || ch == '"' {
            break;
        }
        end = start + offset + ch.len_utf8();
    }

    // Trailing punctuation: ".,;:!?" is sentence punctuation, and a ')' is
    // only part of the URL when it balances an earlier '(' (wiki-style URLs).
    while end > start {
        let last = text[start..end].chars().next_back().unwrap();
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' => true,
            ')' => {
                let slice = &text[start..end];
                slice.matches('(').count() < slice.matches(')').count()
            }
            ']' => true,
            _ => false,
        };
        if !drop {
            break;
        }
        end -= last.len_utf8();
    }
    end
}

/// Apply non-overlapping edits back-to-front so earlier offsets stay valid.
fn apply_edits(src: &str, mut edits: Vec<Edit>) -> String {
    if edits.is_empty() {
        return src.to_string();
    }
    edits.sort_by_key(|e| e.range.start);

    let mut out = String::with_capacity(src.len());
    let mut cursor = 0usize;
    for edit in edits {
        // Defensive: skip anything that overlaps an already-applied edit.
        if edit.range.start < cursor {
            continue;
        }
        out.push_str(&src[cursor..edit.range.start]);
        out.push_str(&edit.replacement);
        cursor = edit.range.end;
    }
    out.push_str(&src[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render exactly the way `dioxus-genai-chat` does, so assertions are
    /// about what actually reaches the DOM.
    fn render(src: &str) -> String {
        let mut out = String::new();
        pulldown_cmark::html::push_html(&mut out, Parser::new_ext(src, parser_options()));
        out
    }

    fn pipeline(src: &str) -> String {
        render(&prepare(&ammonia::clean(src)))
    }

    // ── Autolinking (the reported bug) ──────────────────────────────────

    /// The reported symptom: models emit `**https://…**`, which rendered as
    /// bold plain text with no link.
    #[test]
    fn bold_wrapped_url_becomes_a_link() {
        let html = pipeline("Check **https://example.com/page** for details");
        assert!(
            html.contains(r#"<a href="https://example.com/page">"#),
            "expected a link, got: {html}"
        );
        assert!(html.contains("<strong>"), "bold styling is preserved");
    }

    #[test]
    fn bare_url_becomes_a_link() {
        let html = pipeline("See https://example.com/page for details");
        assert!(
            html.contains(r#"<a href="https://example.com/page">"#),
            "{html}"
        );
        assert!(html.contains(">https://example.com/page</a>"), "{html}");
    }

    #[test]
    fn http_and_trailing_punctuation() {
        let html = pipeline("Go to http://example.com/a.");
        assert!(html.contains(r#"href="http://example.com/a""#), "{html}");
        assert!(
            html.ends_with(".</p>\n"),
            "sentence period stays out: {html}"
        );
    }

    #[test]
    fn balanced_parens_stay_in_the_url() {
        let html = pipeline("see https://en.wikipedia.org/wiki/Rust_(language) ok");
        assert!(
            html.contains(r#"href="https://en.wikipedia.org/wiki/Rust_(language)""#),
            "the whole URL, underscore and balanced parens included: {html}"
        );
    }

    #[test]
    fn url_in_parentheses_does_not_swallow_the_closer() {
        let html = pipeline("(see https://example.com/a) done");
        assert!(html.contains(r#"href="https://example.com/a""#), "{html}");
        assert!(
            html.contains(") done"),
            "closing paren stays as prose: {html}"
        );
    }

    // ── Things that must NOT be autolinked ──────────────────────────────

    #[test]
    fn urls_in_code_are_left_alone() {
        let inline = pipeline("Use `https://example.com` here");
        assert!(!inline.contains("<a "), "inline code untouched: {inline}");

        let fenced = pipeline("```\nhttps://example.com\n```");
        assert!(!fenced.contains("<a "), "code fence untouched: {fenced}");
    }

    #[test]
    fn existing_markdown_links_are_not_double_wrapped() {
        let html = pipeline("[example](https://example.com/page)");
        assert_eq!(html.matches("<a ").count(), 1, "exactly one anchor: {html}");
        assert!(html.contains(">example</a>"), "link text preserved: {html}");
    }

    #[test]
    fn a_url_used_as_link_text_is_not_nested() {
        let html = pipeline("[https://example.com](https://example.com)");
        assert_eq!(html.matches("<a ").count(), 1, "no nested anchor: {html}");
    }

    #[test]
    fn scheme_needs_a_word_boundary() {
        let html = pipeline("notahttps://example.com");
        assert!(!html.contains("<a "), "no link mid-word: {html}");
    }

    // ── Unsafe destinations (the security hole) ─────────────────────────

    /// Regression: `ammonia` runs on the markdown *source*, so it never saw
    /// the anchor pulldown-cmark built from markdown link syntax.
    #[test]
    fn javascript_links_are_neutralised() {
        for src in [
            "[click me](javascript:alert(1))",
            "[click me](  javascript:alert(1))",
            "[click me](JaVaScRiPt:alert(1))",
        ] {
            let html = pipeline(src);
            // The literal characters may remain as inert prose; what must not
            // exist is an anchor whose href carries the scheme.
            assert!(
                !html.contains(r#"href="javascript:"#),
                "javascript href survived for {src:?}: {html}"
            );
            assert!(
                !html.contains("<a "),
                "no anchor emitted for {src:?}: {html}"
            );
            assert!(
                html.contains("click me"),
                "text is kept for {src:?}: {html}"
            );
        }
    }

    #[test]
    fn other_dangerous_schemes_are_neutralised() {
        for src in [
            "[x](data:text/html;base64,PHNjcmlwdD4=)",
            "[x](vbscript:msgbox)",
            "[x](file:///etc/passwd)",
        ] {
            let html = pipeline(src);
            assert!(!html.contains("<a "), "no anchor for {src:?}: {html}");
        }
    }

    #[test]
    fn dangerous_image_sources_are_neutralised() {
        let html = pipeline("![img](javascript:alert(1))");
        assert!(!html.contains(r#"src="javascript:"#), "{html}");
        assert!(!html.contains("<img"), "{html}");
    }

    /// Alt text must be left exactly as written. Autolinking inside it
    /// rewrote the source to `![see [url](<url>)](…)`, which only rendered
    /// correctly because CommonMark flattens alt text back to plain text.
    #[test]
    fn url_in_image_alt_text_is_left_alone() {
        let src = "![see https://example.com](https://safe.example/i.png)";
        assert_eq!(prepare(src), src, "the source must not be rewritten");

        let html = pipeline(src);
        assert!(
            html.contains(r#"<img src="https://safe.example/i.png""#),
            "{html}"
        );
        assert!(
            !html.contains("<a "),
            "alt text must not be autolinked: {html}"
        );
        assert!(html.contains(r#"alt="see https://example.com""#), "{html}");
    }

    /// An image is not a link, so prose *after* it still autolinks — proving
    /// the depth counter is decremented rather than leaking.
    #[test]
    fn autolinking_resumes_after_an_image() {
        let html = pipeline("![i](https://safe.example/i.png) then https://example.com/x");
        assert!(html.contains("<img "), "{html}");
        assert!(
            html.contains(r#"<a href="https://example.com/x""#),
            "{html}"
        );
    }

    #[test]
    fn safe_destinations_still_work() {
        for (src, needle) in [
            ("[a](https://example.com)", r#"href="https://example.com""#),
            ("[b](http://example.com)", r#"href="http://example.com""#),
            (
                "[c](mailto:x@example.com)",
                r#"href="mailto:x@example.com""#,
            ),
            ("[d](/relative/path)", r#"href="/relative/path""#),
            ("[e](#anchor)", r##"href="#anchor""##),
            (
                "![f](https://example.com/i.png)",
                r#"src="https://example.com/i.png""#,
            ),
        ] {
            let html = pipeline(src);
            assert!(html.contains(needle), "{src} → {html}");
        }
    }

    #[test]
    fn a_neutralised_link_cannot_be_reparsed() {
        // The escaped brackets must not re-form a link on the second parse.
        let html = pipeline("[click](javascript:alert(1))");
        assert!(!html.contains("<a "), "{html}");
        assert!(!html.contains(r#"href="#), "no href at all: {html}");
    }

    // ── Structural safety ───────────────────────────────────────────────

    #[test]
    fn plain_text_is_unchanged() {
        assert_eq!(prepare("just some prose"), "just some prose");
        assert_eq!(prepare(""), "");
    }

    #[test]
    fn multibyte_text_is_not_corrupted() {
        let src = "日本語 https://example.com/ページ の説明";
        let out = prepare(src);
        assert!(out.contains("日本語"), "{out}");
        assert!(out.contains("の説明"), "{out}");
        let html = render(&out);
        assert!(html.contains("<a "), "{html}");
    }

    #[test]
    fn multiple_urls_in_one_paragraph() {
        let html = pipeline("first https://a.example second https://b.example end");
        assert_eq!(html.matches("<a ").count(), 2, "{html}");
        assert!(html.contains("end"), "trailing prose survives: {html}");
    }

    #[test]
    fn raw_html_is_still_sanitised_by_ammonia() {
        let html = pipeline("<script>alert(1)</script>hello");
        assert!(!html.contains("<script"), "{html}");
    }
}
