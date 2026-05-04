//! Lightweight markdown → HTML rendering for assistant messages.
//!
//! We use `pulldown-cmark` directly and run a *minimal* sanitisation pass
//! (strip `<script>`/`<iframe>`/`<style>`/`<object>` blocks) before injecting
//! the result into the page. Inputs come from a paired RustyClaw gateway, but
//! the desktop client should still avoid blindly trusting them.

use pulldown_cmark::{Options, Parser, html};

/// Render a markdown string into a sanitised HTML fragment.
pub fn render(input: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_SMART_PUNCTUATION);

    let parser = Parser::new_ext(input, opts);

    let mut out = String::with_capacity(input.len());
    html::push_html(&mut out, parser);

    sanitise(&out)
}

/// Strip the small set of HTML constructs that have no business appearing
/// inside a chat bubble. Everything else is left as-is so markdown formatting
/// (links, lists, code blocks, etc.) renders normally.
fn sanitise(html: &str) -> String {
    const FORBIDDEN: &[&str] = &["script", "iframe", "object", "embed", "style"];

    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'<'
            && let Some(forbidden) = FORBIDDEN
                .iter()
                .find(|tag| starts_with_tag(&bytes[i..], tag.as_bytes()))
        {
            // Skip until matching closing tag (case-insensitive) or end of input.
            let close = format!("</{}>", forbidden);
            if let Some(idx) = find_case_insensitive(&html[i..], &close) {
                i += idx + close.len();
            } else {
                // Drop the rest if we can't find a closing tag.
                break;
            }
            continue;
        }
        out.push(byte as char);
        i += 1;
    }

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
}
