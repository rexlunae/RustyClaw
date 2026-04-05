//! Markdown-to-platform conversion utilities.
//!
//! Functions for converting standard Markdown to platform-specific formats.

/// Telegram message character limit.
pub const TELEGRAM_MAX_LEN: usize = 4096;
/// Slack message character limit.
pub const SLACK_MAX_LEN: usize = 40_000;

fn escape_html(s: impl AsRef<str>) -> String {
    let s = s.as_ref();
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// Convert Markdown to Telegram HTML.
///
/// Telegram supports `<b>`, `<i>`, `<s>`, `<code>`, `<pre>`, `<a href="">`.
pub fn markdown_to_telegram_html(md: impl AsRef<str>) -> String {
    let escaped = escape_html(md);
    let mut out = String::with_capacity(escaped.len() + 64);
    let mut i = 0;

    while i < escaped.len() {
        // Fenced code block: ```
        if escaped.get(i..i + 3) == Some("```") {
            i += 3;
            let start = i;
            let mut code_end = None;
            let mut j = i;
            while j <= escaped.len().saturating_sub(3) {
                if escaped.get(j..j + 3) == Some("```") {
                    code_end = Some(j);
                    break;
                }
                // advance by one char
                let ch_len = escaped[j..].chars().next().map_or(1, |c| c.len_utf8());
                j += ch_len;
            }
            if let Some(end) = code_end {
                let content = &escaped[start..end];
                let (lang, code) = if let Some(nl) = content.find('\n') {
                    let maybe_lang = content[..nl].trim();
                    if !maybe_lang.is_empty() && !maybe_lang.contains(' ') {
                        (maybe_lang, &content[nl + 1..])
                    } else {
                        ("", content)
                    }
                } else {
                    ("", content)
                };
                if lang.is_empty() {
                    out.push_str("<pre>");
                    out.push_str(code);
                    out.push_str("</pre>");
                } else {
                    out.push_str("<pre><code class=\"language-");
                    out.push_str(lang);
                    out.push_str("\">");
                    out.push_str(code);
                    out.push_str("</code></pre>");
                }
                i = end + 3;
            } else {
                out.push_str("```");
            }
            continue;
        }

        // Inline code: `
        if escaped.get(i..i + 1) == Some("`") {
            i += 1;
            let start = i;
            while i < escaped.len() && escaped.get(i..i + 1) != Some("`") {
                let ch_len = escaped[i..].chars().next().map_or(1, |c| c.len_utf8());
                i += ch_len;
            }
            out.push_str("<code>");
            out.push_str(&escaped[start..i]);
            out.push_str("</code>");
            if i < escaped.len() {
                i += 1; // skip closing `
            }
            continue;
        }

        // Bold: **
        if escaped.get(i..i + 2) == Some("**") {
            i += 2;
            let start = i;
            if let Some(rel) = escaped[i..].find("**") {
                let end = i + rel;
                out.push_str("<b>");
                out.push_str(&escaped[start..end]);
                out.push_str("</b>");
                i = end + 2;
            } else {
                out.push_str("**");
            }
            continue;
        }

        // Strikethrough: ~~
        if escaped.get(i..i + 2) == Some("~~") {
            i += 2;
            let start = i;
            if let Some(rel) = escaped[i..].find("~~") {
                let end = i + rel;
                out.push_str("<s>");
                out.push_str(&escaped[start..end]);
                out.push_str("</s>");
                i = end + 2;
            } else {
                out.push_str("~~");
            }
            continue;
        }

        // Italic: * (single)
        if escaped.get(i..i + 1) == Some("*") {
            i += 1;
            let start = i;
            if let Some(rel) = escaped[i..].find('*') {
                let end = i + rel;
                out.push_str("<i>");
                out.push_str(&escaped[start..end]);
                out.push_str("</i>");
                i = end + 1;
            } else {
                out.push('*');
            }
            continue;
        }

        // Italic: _text_
        if escaped.get(i..i + 1) == Some("_") {
            i += 1;
            let start = i;
            if let Some(rel) = escaped[i..].find('_') {
                let end = i + rel;
                out.push_str("<i>");
                out.push_str(&escaped[start..end]);
                out.push_str("</i>");
                i = end + 1;
            } else {
                out.push('_');
            }
            continue;
        }

        // Link: [text](url)
        if escaped.get(i..i + 1) == Some("[") {
            if let Some(close_bracket_rel) = escaped[i..].find("](") {
                let text_end = i + close_bracket_rel;
                let url_start = text_end + 2;
                if let Some(close_paren_rel) = escaped[url_start..].find(')') {
                    let url_end = url_start + close_paren_rel;
                    let link_text = &escaped[i + 1..text_end];
                    let url = &escaped[url_start..url_end];
                    out.push_str("<a href=\"");
                    out.push_str(url);
                    out.push_str("\">");
                    out.push_str(link_text);
                    out.push_str("</a>");
                    i = url_end + 1;
                    continue;
                }
            }
        }

        // Regular char
        let ch = escaped[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

/// Convert Markdown to Slack mrkdwn format.
pub fn markdown_to_slack(text: impl AsRef<str>) -> String {
    let text = text.as_ref();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();
    let len = text.len();

    while i < len {
        // Bold: **text** → *text*
        if text.get(i..i + 2) == Some("**") {
            i += 2;
            let start = i;
            if let Some(rel) = text[i..].find("**") {
                let end = i + rel;
                out.push('*');
                out.push_str(&text[start..end]);
                out.push('*');
                i = end + 2;
            } else {
                out.push_str("**");
            }
            continue;
        }

        // Strikethrough: ~~text~~ → ~text~
        if text.get(i..i + 2) == Some("~~") {
            i += 2;
            let start = i;
            if let Some(rel) = text[i..].find("~~") {
                let end = i + rel;
                out.push('~');
                out.push_str(&text[start..end]);
                out.push('~');
                i = end + 2;
            } else {
                out.push_str("~~");
            }
            continue;
        }

        // Link: [text](url) → <url|text>
        if text.get(i..i + 1) == Some("[") {
            if let Some(close_bracket_rel) = text[i..].find("](") {
                let text_end = i + close_bracket_rel;
                let url_start = text_end + 2;
                if let Some(close_paren_rel) = text[url_start..].find(')') {
                    let url_end = url_start + close_paren_rel;
                    let link_text = &text[i + 1..text_end];
                    let url = &text[url_start..url_end];
                    out.push('<');
                    out.push_str(url);
                    out.push('|');
                    out.push_str(link_text);
                    out.push('>');
                    i = url_end + 1;
                    continue;
                }
            }
        }

        // Header: # at line start → *Header*
        if (i == 0 || bytes.get(i.saturating_sub(1)) == Some(&b'\n'))
            && text.get(i..i + 1) == Some("#")
        {
            // Count heading level (we just flatten to bold)
            let mut hashes = 0;
            let mut j = i;
            while text.get(j..j + 1) == Some("#") {
                hashes += 1;
                j += 1;
            }
            if hashes > 0 && text.get(j..j + 1) == Some(" ") {
                j += 1; // skip space
                let line_end = text[j..].find('\n').map_or(text.len(), |p| j + p);
                out.push('*');
                out.push_str(&text[j..line_end]);
                out.push('*');
                i = line_end;
                continue;
            }
        }

        // Regular char
        let ch = text[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

/// Find the largest char boundary at or before `pos` in `s`.
fn floor_char_boundary(s: &str, pos: usize) -> usize {
    let mut e = pos.min(s.len());
    while e > 0 && !s.is_char_boundary(e) {
        e -= 1;
    }
    e
}

/// Convert markdown to Telegram HTML then split into chunks of at most `max_len` bytes.
pub fn chunk_markdown_html(md: impl AsRef<str>, max_len: usize) -> Vec<String> {
    let html = markdown_to_telegram_html(md);
    if html.len() <= max_len {
        return vec![html];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();

    for line in html.split('\n') {
        let with_newline = if current.is_empty() {
            line.to_string()
        } else {
            format!("\n{}", line)
        };

        if current.len() + with_newline.len() > max_len {
            if !current.is_empty() {
                chunks.push(current.clone());
                current = line.to_string();
            } else {
                // Single line exceeds max_len, force split
                let mut pos = 0;
                while pos < line.len() {
                    let end = floor_char_boundary(line, pos + max_len);
                    let end = if end <= pos { pos + 1 } else { end };
                    let end = end.min(line.len());
                    chunks.push(line[pos..end].to_string());
                    pos = end;
                }
            }
        } else {
            current.push_str(&with_newline);
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_html() {
        assert_eq!(escape_html("a & b"), "a &amp; b");
        assert_eq!(escape_html("<tag>"), "&lt;tag&gt;");
    }

    #[test]
    fn test_telegram_bold() {
        let result = markdown_to_telegram_html("**hello**");
        assert_eq!(result, "<b>hello</b>");
    }

    #[test]
    fn test_telegram_italic_star() {
        let result = markdown_to_telegram_html("*hello*");
        assert_eq!(result, "<i>hello</i>");
    }

    #[test]
    fn test_telegram_strike() {
        let result = markdown_to_telegram_html("~~hello~~");
        assert_eq!(result, "<s>hello</s>");
    }

    #[test]
    fn test_telegram_inline_code() {
        let result = markdown_to_telegram_html("`code`");
        assert_eq!(result, "<code>code</code>");
    }

    #[test]
    fn test_telegram_link() {
        let result = markdown_to_telegram_html("[click](https://example.com)");
        assert_eq!(result, "<a href=\"https://example.com\">click</a>");
    }

    #[test]
    fn test_telegram_html_escape() {
        let result = markdown_to_telegram_html("a & b");
        assert!(result.contains("&amp;"));
    }

    #[test]
    fn test_slack_bold() {
        assert_eq!(markdown_to_slack("**hello**"), "*hello*");
    }

    #[test]
    fn test_slack_strike() {
        assert_eq!(markdown_to_slack("~~hello~~"), "~hello~");
    }

    #[test]
    fn test_slack_link() {
        assert_eq!(
            markdown_to_slack("[click](https://example.com)"),
            "<https://example.com|click>"
        );
    }

    #[test]
    fn test_slack_header() {
        assert_eq!(markdown_to_slack("# Hello"), "*Hello*");
    }

    #[test]
    fn test_chunk_small() {
        let chunks = chunk_markdown_html("hello", 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "hello");
    }

    #[test]
    fn test_chunk_split() {
        let long_md = "line1\nline2\nline3";
        let chunks = chunk_markdown_html(long_md, 8);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 8);
        }
    }
}
