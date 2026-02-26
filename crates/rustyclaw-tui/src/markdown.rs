//! Simple markdown to styled text conversion for TUI rendering.
//!
//! Parses basic markdown and returns styled segments that can be rendered
//! with iocraft Text elements.

#[allow(unused_imports)]
use iocraft::prelude::*;

/// A styled text segment.
#[derive(Debug, Clone)]
pub struct StyledSegment {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub header_level: u8, // 0 = not a header, 1-6 = h1-h6
}

impl StyledSegment {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bold: false,
            italic: false,
            code: false,
            header_level: 0,
        }
    }

    pub fn bold(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bold: true,
            italic: false,
            code: false,
            header_level: 0,
        }
    }

    pub fn code(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            bold: false,
            italic: false,
            code: true,
            header_level: 0,
        }
    }

    pub fn header(text: impl Into<String>, level: u8) -> Self {
        Self {
            text: text.into(),
            bold: true,
            italic: false,
            code: false,
            header_level: level,
        }
    }
}

/// Parse markdown text into styled segments.
///
/// Supports:
/// - **bold** and __bold__
/// - *italic* and _italic_
/// - `inline code`
/// - # Headers (at line start)
/// - Lists (-, *, numbered) — rendered with bullet/number
///
/// Does NOT support (rendered as plain text):
/// - Code blocks (```) — kept as-is with backticks stripped
/// - Links, images
/// - Tables
pub fn parse_markdown(input: &str) -> Vec<StyledSegment> {
    let mut segments = Vec::new();

    for line in input.lines() {
        // Check for headers
        if let Some(header) = parse_header(line) {
            segments.push(header);
            segments.push(StyledSegment::plain("\n"));
            continue;
        }

        // Parse inline formatting
        parse_inline(line, &mut segments);
        segments.push(StyledSegment::plain("\n"));
    }

    // Remove trailing newline
    if let Some(last) = segments.last() {
        if last.text == "\n" {
            segments.pop();
        }
    }

    segments
}

/// Parse a header line (# Header).
fn parse_header(line: &str) -> Option<StyledSegment> {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();

    if hashes > 0 && hashes <= 6 {
        let rest = trimmed[hashes..].trim_start();
        if !rest.is_empty() || hashes == trimmed.len() {
            return Some(StyledSegment::header(rest, hashes as u8));
        }
    }

    None
}

/// Parse inline formatting (**bold**, *italic*, `code`).
fn parse_inline(text: &str, segments: &mut Vec<StyledSegment>) {
    let mut chars = text.chars().peekable();
    let mut current = String::new();

    while let Some(c) = chars.next() {
        match c {
            // Bold: ** or __
            '*' | '_' if chars.peek() == Some(&c) => {
                // Flush current
                if !current.is_empty() {
                    segments.push(StyledSegment::plain(std::mem::take(&mut current)));
                }

                chars.next(); // consume second marker
                let marker = c;

                // Collect until closing **
                let mut bold_text = String::new();
                while let Some(bc) = chars.next() {
                    if bc == marker && chars.peek() == Some(&marker) {
                        chars.next(); // consume second marker
                        break;
                    }
                    bold_text.push(bc);
                }

                if !bold_text.is_empty() {
                    segments.push(StyledSegment::bold(bold_text));
                }
            }

            // Inline code: `code`
            '`' => {
                // Flush current
                if !current.is_empty() {
                    segments.push(StyledSegment::plain(std::mem::take(&mut current)));
                }

                // Check for code block (```)
                if chars.peek() == Some(&'`') {
                    chars.next();
                    if chars.peek() == Some(&'`') {
                        chars.next();
                        // Code block — collect until closing ```
                        let mut code_text = String::new();
                        let mut backtick_count = 0;
                        for cc in chars.by_ref() {
                            if cc == '`' {
                                backtick_count += 1;
                                if backtick_count == 3 {
                                    break;
                                }
                            } else {
                                // Add any accumulated backticks that weren't closing
                                for _ in 0..backtick_count {
                                    code_text.push('`');
                                }
                                backtick_count = 0;
                                code_text.push(cc);
                            }
                        }
                        if !code_text.is_empty() {
                            segments.push(StyledSegment::code(code_text));
                        }
                        continue;
                    }
                }

                // Single backtick — inline code
                let mut code_text = String::new();
                for cc in chars.by_ref() {
                    if cc == '`' {
                        break;
                    }
                    code_text.push(cc);
                }

                if !code_text.is_empty() {
                    segments.push(StyledSegment::code(code_text));
                }
            }

            // Italic: single * or _ (not followed by same char)
            '*' | '_' => {
                // Flush current
                if !current.is_empty() {
                    segments.push(StyledSegment::plain(std::mem::take(&mut current)));
                }

                let marker = c;
                let mut italic_text = String::new();

                for ic in chars.by_ref() {
                    if ic == marker {
                        break;
                    }
                    italic_text.push(ic);
                }

                // For now, render italic as plain (most terminals don't support italic)
                if !italic_text.is_empty() {
                    // Could add italic: true to StyledSegment if terminal supports it
                    segments.push(StyledSegment::plain(italic_text));
                }
            }

            _ => {
                current.push(c);
            }
        }
    }

    // Flush remaining
    if !current.is_empty() {
        segments.push(StyledSegment::plain(current));
    }
}

/// Render markdown as a single string with ANSI escape codes.
///
/// This is a fallback for when we can't use multiple Text elements.
pub fn render_ansi(input: &str) -> String {
    let segments = parse_markdown(input);
    let mut output = String::new();

    for seg in segments {
        if seg.bold || seg.header_level > 0 {
            output.push_str("\x1b[1m"); // Bold
        }
        if seg.code {
            output.push_str("\x1b[36m"); // Cyan for code
        }

        output.push_str(&seg.text);

        if seg.bold || seg.header_level > 0 || seg.code {
            output.push_str("\x1b[0m"); // Reset
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_text() {
        let segments = parse_markdown("Hello world");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "Hello world");
        assert!(!segments[0].bold);
    }

    #[test]
    fn test_bold() {
        let segments = parse_markdown("Hello **bold** world");
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "Hello ");
        assert_eq!(segments[1].text, "bold");
        assert!(segments[1].bold);
        assert_eq!(segments[2].text, " world");
    }

    #[test]
    fn test_inline_code() {
        let segments = parse_markdown("Use `cargo build` here");
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[1].text, "cargo build");
        assert!(segments[1].code);
    }

    #[test]
    fn test_header() {
        let segments = parse_markdown("# Hello\nWorld");
        assert_eq!(segments[0].header_level, 1);
        assert_eq!(segments[0].text, "Hello");
    }

    #[test]
    fn test_ansi_render() {
        let output = render_ansi("Hello **bold** and `code`");
        assert!(output.contains("\x1b[1m")); // Bold
        assert!(output.contains("\x1b[36m")); // Cyan
        assert!(output.contains("\x1b[0m")); // Reset
    }
}
