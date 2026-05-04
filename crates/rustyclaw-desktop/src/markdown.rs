//! Lightweight markdown → HTML rendering for assistant messages.
//!
//! Output is injected into the desktop webview via `dangerous_inner_html`,
//! so the renderer treats incoming markdown as untrusted and applies two
//! layers of defence:
//!
//! 1. **Strip all raw HTML** at the `pulldown-cmark` event level. Assistant
//!    messages don't need to inject custom HTML, so we drop every
//!    `Event::Html` / `Event::InlineHtml` event before serialising. This
//!    eliminates injected `<img onerror=...>`, `<svg onload=...>`, inline
//!    event handlers, etc. *before* they ever reach the HTML output.
//! 2. **Rewrite dangerous link URLs**. Markdown link/image targets like
//!    `[click](javascript:alert(1))` or `![](data:text/html,<script>...)`
//!    are still expressed as cmark events (`Tag::Link`/`Tag::Image`), so
//!    we walk the event stream and replace any non-allow-listed URL
//!    scheme with `#`.
//!
//! We also keep a small belt-and-suspenders pass that strips a few
//! forbidden tag blocks (`script`, `iframe`, `object`, `embed`, `style`)
//! after rendering, in case future cmark changes ever leak raw HTML.

use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, html};

/// Render a markdown string into a sanitised HTML fragment.
pub fn render(input: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_SMART_PUNCTUATION);

    let parser = Parser::new_ext(input, opts);
    let safe = SafeEventFilter::new(parser);

    let mut out = String::with_capacity(input.len());
    html::push_html(&mut out, safe);

    sanitise(&out)
}

/// Forbidden HTML tags whose content (not just the tags themselves) must be
/// stripped — `pulldown-cmark` may emit these as separate open/close
/// `Event::InlineHtml` events with plain `Event::Text` between them, so we
/// need a small state machine to drop the text in the middle too.
const FORBIDDEN_TAGS: &[&str] = &["script", "style", "iframe", "object", "embed", "noscript"];

/// Stateful event-stream wrapper that:
///   * drops every raw-HTML event (`Event::Html` / `Event::InlineHtml`),
///   * additionally drops every event (including plain text) emitted
///     between an opening forbidden tag (e.g. `<script>`) and its closing
///     counterpart (`</script>`),
///   * rewrites link/image URLs that use non-allow-listed schemes.
struct SafeEventFilter<'a, I: Iterator<Item = Event<'a>>> {
    inner: I,
    skipping: Option<&'static str>,
}

impl<'a, I: Iterator<Item = Event<'a>>> SafeEventFilter<'a, I> {
    fn new(inner: I) -> Self {
        Self {
            inner,
            skipping: None,
        }
    }
}

impl<'a, I: Iterator<Item = Event<'a>>> Iterator for SafeEventFilter<'a, I> {
    type Item = Event<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let event = self.inner.next()?;

            // If we're inside a forbidden block, drop everything until we
            // see the matching closing tag.
            if let Some(tag) = self.skipping {
                if let Event::Html(s) | Event::InlineHtml(s) = &event
                    && is_close_tag(s, tag)
                {
                    self.skipping = None;
                }
                continue;
            }

            match event {
                Event::Html(ref s) | Event::InlineHtml(ref s) => {
                    if let Some(tag) = forbidden_open_tag(s) {
                        self.skipping = Some(tag);
                    }
                    // Drop *all* raw HTML events from the output. The
                    // assistant's markdown should never need to inject
                    // custom HTML, and this is the primary defence
                    // against `<img onerror=...>`-style XSS.
                    continue;
                }
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    title,
                    id,
                }) => {
                    return Some(Event::Start(Tag::Link {
                        link_type,
                        dest_url: sanitise_url(dest_url),
                        title,
                        id,
                    }));
                }
                Event::Start(Tag::Image {
                    link_type,
                    dest_url,
                    title,
                    id,
                }) => {
                    return Some(Event::Start(Tag::Image {
                        link_type,
                        dest_url: sanitise_url_for_image(dest_url, link_type),
                        title,
                        id,
                    }));
                }
                other => return Some(other),
            }
        }
    }
}

/// If `s` looks like an opening tag for one of the forbidden tags
/// (case-insensitive), return the canonical tag name. Otherwise `None`.
fn forbidden_open_tag(s: &str) -> Option<&'static str> {
    let bytes = s.as_bytes();
    // Need at least `<x>`.
    if bytes.len() < 2 || bytes[0] != b'<' || bytes[1] == b'/' {
        return None;
    }
    for tag in FORBIDDEN_TAGS {
        let tb = tag.as_bytes();
        if bytes.len() < tb.len() + 1 {
            continue;
        }
        let mut matches = true;
        for (i, &t) in tb.iter().enumerate() {
            if !bytes[1 + i].eq_ignore_ascii_case(&t) {
                matches = false;
                break;
            }
        }
        if !matches {
            continue;
        }
        let after = bytes.get(1 + tb.len()).copied().unwrap_or(b'\0');
        if matches!(after, b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/') {
            return Some(tag);
        }
    }
    None
}

/// Whether `s` is a closing tag for `tag` (case-insensitive).
fn is_close_tag(s: &str, tag: &str) -> bool {
    let bytes = s.as_bytes();
    let tb = tag.as_bytes();
    // Need at least `</x>`.
    if bytes.len() < tb.len() + 3 || bytes[0] != b'<' || bytes[1] != b'/' {
        return false;
    }
    for (i, &t) in tb.iter().enumerate() {
        if !bytes[2 + i].eq_ignore_ascii_case(&t) {
            return false;
        }
    }
    let after = bytes.get(2 + tb.len()).copied().unwrap_or(b'\0');
    matches!(after, b' ' | b'\t' | b'\n' | b'\r' | b'>')
}

/// Allow-list of URL schemes that are safe to render inside a chat bubble.
const SAFE_SCHEMES: &[&str] = &["http", "https", "mailto", "tel"];

fn sanitise_url(url: CowStr<'_>) -> CowStr<'_> {
    if is_safe_url(url.as_ref()) {
        url
    } else {
        CowStr::Borrowed("#")
    }
}

fn sanitise_url_for_image(url: CowStr<'_>, _link_type: LinkType) -> CowStr<'_> {
    // Images use the same allow-list as links. We deliberately do NOT
    // allow `data:` URIs even for images: they can carry SVG with
    // embedded scripts.
    sanitise_url(url)
}

fn is_safe_url(url: &str) -> bool {
    let trimmed = url.trim_start();

    // Relative URLs (no scheme) and fragment-only URLs are safe.
    let scheme_end = match trimmed.find(':') {
        Some(idx) => idx,
        None => return true,
    };

    // A leading `/`, `?`, `#`, etc. before any `:` means this is a
    // path/query/fragment, not a scheme.
    let before_colon = &trimmed[..scheme_end];
    if before_colon.is_empty()
        || before_colon
            .chars()
            .any(|c| !c.is_ascii_alphanumeric() && c != '+' && c != '-' && c != '.')
    {
        return true;
    }

    let scheme_lower = before_colon.to_ascii_lowercase();
    SAFE_SCHEMES.iter().any(|s| *s == scheme_lower)
}

/// Strip the small set of HTML constructs that have no business appearing
/// inside a chat bubble. Everything else is left as-is so markdown formatting
/// (links, lists, code blocks, etc.) renders normally.
///
/// Forbidden tags are matched against the ASCII portion of the bytes (HTML
/// tag names are always ASCII), but the kept regions are copied as contiguous
/// `&str` slices so multi-byte characters (smart punctuation from
/// `ENABLE_SMART_PUNCTUATION`, accented text, emoji, etc.) survive intact.
fn sanitise(html: &str) -> String {
    const FORBIDDEN: &[&str] = &["script", "iframe", "object", "embed", "style"];

    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut copy_from = 0usize;
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'<'
            && let Some(forbidden) = FORBIDDEN
                .iter()
                .find(|tag| starts_with_tag(&bytes[i..], tag.as_bytes()))
        {
            // Flush the safe slice before this forbidden tag.
            out.push_str(&html[copy_from..i]);

            // Skip until matching closing tag (case-insensitive) or end of input.
            let close = format!("</{}>", forbidden);
            if let Some(idx) = find_case_insensitive(&html[i..], &close) {
                i += idx + close.len();
                copy_from = i;
                continue;
            } else {
                // Drop the rest if we can't find a closing tag.
                return out;
            }
        }
        i += 1;
    }

    // Flush any remaining tail.
    out.push_str(&html[copy_from..]);
    out
}

fn starts_with_tag(bytes: &[u8], tag: &[u8]) -> bool {
    // Match `<tag` or `<tag ...` or `<tag>` case-insensitively, with no
    // intervening characters.
    if bytes.len() < tag.len() + 1 || bytes[0] != b'<' {
        return false;
    }
    for (i, &t) in tag.iter().enumerate() {
        let b = bytes[1 + i];
        if !b.eq_ignore_ascii_case(&t) {
            return false;
        }
    }
    let after = bytes.get(1 + tag.len()).copied().unwrap_or(b'\0');
    matches!(after, b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/')
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let haystack_lower = haystack.to_ascii_lowercase();
    let needle_lower = needle.to_ascii_lowercase();
    haystack_lower.find(&needle_lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_paragraph() {
        let html = render("Hello **world**");
        assert!(html.contains("<strong>world</strong>"));
    }

    #[test]
    fn fenced_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let html = render(md);
        assert!(html.contains("<pre>"));
        assert!(html.contains("fn main()"));
    }

    #[test]
    fn strips_script_tags() {
        let md = "Hi <script>alert('x')</script> there";
        let html = render(md);
        assert!(!html.contains("<script"));
        assert!(!html.contains("alert"));
    }

    #[test]
    fn strips_iframe_tags() {
        let md = "<iframe src='x'></iframe>";
        let html = render(md);
        assert!(!html.contains("<iframe"));
    }

    #[test]
    fn keeps_inline_code() {
        let html = render("use `foo` here");
        assert!(html.contains("<code>foo</code>"));
    }

    #[test]
    fn preserves_smart_punctuation() {
        // ENABLE_SMART_PUNCTUATION turns straight quotes into curly ones, which
        // are multi-byte UTF-8. Ensure the sanitiser preserves them rather
        // than mangling each byte individually.
        let html = render("He said \"hello\" -- and left...");
        assert!(
            html.contains('\u{201C}') && html.contains('\u{201D}'),
            "expected curly quotes in {html:?}"
        );
        assert!(html.contains('\u{2026}'), "expected ellipsis in {html:?}");
        assert!(html.contains('\u{2013}'), "expected en dash in {html:?}");
    }

    #[test]
    fn preserves_non_ascii_in_paragraphs() {
        let html = render("Café — naïve façade 🦀");
        assert!(html.contains("Café"), "got {html:?}");
        assert!(html.contains("naïve"), "got {html:?}");
        assert!(html.contains("façade"), "got {html:?}");
        assert!(html.contains('🦀'), "got {html:?}");
    }

    #[test]
    fn preserves_non_ascii_around_stripped_tag() {
        let html = render("Hi 🦀 <script>bad()</script> there ©");
        assert!(html.contains('🦀'));
        assert!(html.contains('©'));
        assert!(!html.contains("<script"));
        assert!(!html.contains("bad()"));
    }

    #[test]
    fn strips_inline_event_handlers_via_raw_html() {
        // pulldown-cmark would normally pass these through as raw HTML
        // events. We filter those out before rendering, so neither the
        // tag nor the handler ever reaches the output.
        let cases = [
            "<img src=x onerror=\"alert(1)\">",
            "<svg onload=\"alert(1)\"></svg>",
            "<div onclick=\"alert(1)\">click</div>",
        ];
        for md in cases {
            let html = render(md);
            assert!(
                !html.to_ascii_lowercase().contains("onerror"),
                "onerror leaked: {html:?}"
            );
            assert!(
                !html.to_ascii_lowercase().contains("onload"),
                "onload leaked: {html:?}"
            );
            assert!(
                !html.to_ascii_lowercase().contains("onclick"),
                "onclick leaked: {html:?}"
            );
            assert!(!html.contains("alert(1)"), "alert leaked: {html:?}");
        }
    }

    #[test]
    fn rewrites_javascript_urls_in_links() {
        let html = render("[click](javascript:alert(1))");
        assert!(
            !html.to_ascii_lowercase().contains("javascript:"),
            "javascript: leaked: {html:?}"
        );
        assert!(html.contains("href=\"#\""), "expected href=#, got {html:?}");
    }

    #[test]
    fn rewrites_data_urls_in_images() {
        let html = render("![pwn](data:text/html,<script>alert(1)</script>)");
        assert!(
            !html.to_ascii_lowercase().contains("data:"),
            "data: leaked: {html:?}"
        );
        assert!(!html.contains("alert(1)"), "alert leaked: {html:?}");
    }

    #[test]
    fn keeps_safe_urls() {
        let html = render("[GitHub](https://github.com/foo/bar)");
        assert!(html.contains("href=\"https://github.com/foo/bar\""));

        let html = render("[mail](mailto:a@example.com)");
        assert!(html.contains("href=\"mailto:a@example.com\""));

        let html = render("[anchor](#section)");
        assert!(html.contains("href=\"#section\""));

        let html = render("[relative](./other)");
        assert!(html.contains("href=\"./other\""));
    }

    #[test]
    fn case_insensitive_javascript_scheme() {
        let html = render("[x](JaVaScRiPt:alert(1))");
        assert!(
            !html.to_ascii_lowercase().contains("javascript:"),
            "got {html:?}"
        );
    }
}
