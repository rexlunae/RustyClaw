//! Rich text representation and format conversion.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// A node in a rich text tree.
#[derive(Debug, Clone)]
pub enum RichTextNode {
    Plain(String),
    Bold(Vec<RichTextNode>),
    Italic(Vec<RichTextNode>),
    Strikethrough(Vec<RichTextNode>),
    Code(String),
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    Link {
        url: String,
        text: Vec<RichTextNode>,
    },
    Mention {
        id: String,
        name: String,
    },
    Emoji(String),
    Paragraph(Vec<RichTextNode>),
    ListItem(Vec<RichTextNode>),
}

/// A rich text document as a sequence of nodes.
pub struct RichText(pub Vec<RichTextNode>);

impl RichTextNode {
    fn to_plain_text(&self) -> String {
        match self {
            RichTextNode::Plain(s) => s.clone(),
            RichTextNode::Bold(children)
            | RichTextNode::Italic(children)
            | RichTextNode::Strikethrough(children)
            | RichTextNode::Paragraph(children)
            | RichTextNode::ListItem(children) => {
                children.iter().map(|n| n.to_plain_text()).collect()
            }
            RichTextNode::Code(s) => s.clone(),
            RichTextNode::CodeBlock { code, .. } => code.clone(),
            RichTextNode::Link { text, .. } => text.iter().map(|n| n.to_plain_text()).collect(),
            RichTextNode::Mention { name, .. } => format!("@{}", name),
            RichTextNode::Emoji(e) => e.clone(),
        }
    }

    fn to_markdown(&self) -> String {
        match self {
            RichTextNode::Plain(s) => s.clone(),
            RichTextNode::Bold(children) => {
                format!(
                    "**{}**",
                    children.iter().map(|n| n.to_markdown()).collect::<String>()
                )
            }
            RichTextNode::Italic(children) => {
                format!(
                    "*{}*",
                    children.iter().map(|n| n.to_markdown()).collect::<String>()
                )
            }
            RichTextNode::Strikethrough(children) => {
                format!(
                    "~~{}~~",
                    children.iter().map(|n| n.to_markdown()).collect::<String>()
                )
            }
            RichTextNode::Code(s) => format!("`{}`", s),
            RichTextNode::CodeBlock { language, code } => {
                if let Some(lang) = language {
                    format!("```{}\n{}\n```", lang, code)
                } else {
                    format!("```\n{}\n```", code)
                }
            }
            RichTextNode::Link { url, text } => {
                format!(
                    "[{}]({})",
                    text.iter().map(|n| n.to_markdown()).collect::<String>(),
                    url
                )
            }
            RichTextNode::Mention { name, .. } => format!("@{}", name),
            RichTextNode::Emoji(e) => e.clone(),
            RichTextNode::Paragraph(children) | RichTextNode::ListItem(children) => {
                children.iter().map(|n| n.to_markdown()).collect()
            }
        }
    }

    fn to_matrix_html(&self) -> String {
        match self {
            RichTextNode::Plain(s) => html_escape(s),
            RichTextNode::Bold(children) => {
                format!(
                    "<b>{}</b>",
                    children
                        .iter()
                        .map(|n| n.to_matrix_html())
                        .collect::<String>()
                )
            }
            RichTextNode::Italic(children) => {
                format!(
                    "<i>{}</i>",
                    children
                        .iter()
                        .map(|n| n.to_matrix_html())
                        .collect::<String>()
                )
            }
            RichTextNode::Strikethrough(children) => {
                format!(
                    "<del>{}</del>",
                    children
                        .iter()
                        .map(|n| n.to_matrix_html())
                        .collect::<String>()
                )
            }
            RichTextNode::Code(s) => format!("<code>{}</code>", html_escape(s)),
            RichTextNode::CodeBlock { language, code } => {
                if let Some(lang) = language {
                    format!(
                        "<pre><code class=\"language-{}\">{}</code></pre>",
                        lang,
                        html_escape(code)
                    )
                } else {
                    format!("<pre>{}</pre>", html_escape(code))
                }
            }
            RichTextNode::Link { url, text } => {
                format!(
                    "<a href=\"{}\">{}</a>",
                    html_escape(url),
                    text.iter().map(|n| n.to_matrix_html()).collect::<String>()
                )
            }
            RichTextNode::Mention { name, .. } => format!("@{}", html_escape(name)),
            RichTextNode::Emoji(e) => html_escape(e),
            RichTextNode::Paragraph(children) | RichTextNode::ListItem(children) => {
                children.iter().map(|n| n.to_matrix_html()).collect()
            }
        }
    }

    fn to_irc_formatted(&self) -> String {
        match self {
            RichTextNode::Plain(s) => s.clone(),
            RichTextNode::Bold(children) => {
                format!(
                    "\x02{}\x02",
                    children
                        .iter()
                        .map(|n| n.to_irc_formatted())
                        .collect::<String>()
                )
            }
            RichTextNode::Italic(children) => {
                format!(
                    "\x1D{}\x1D",
                    children
                        .iter()
                        .map(|n| n.to_irc_formatted())
                        .collect::<String>()
                )
            }
            RichTextNode::Strikethrough(children) => {
                children.iter().map(|n| n.to_irc_formatted()).collect()
            }
            RichTextNode::Code(s) => format!("`{}`", s),
            RichTextNode::CodeBlock { code, .. } => code.clone(),
            RichTextNode::Link { url, text } => {
                format!(
                    "{} ({})",
                    text.iter()
                        .map(|n| n.to_irc_formatted())
                        .collect::<String>(),
                    url
                )
            }
            RichTextNode::Mention { name, .. } => format!("@{}", name),
            RichTextNode::Emoji(e) => e.clone(),
            RichTextNode::Paragraph(children) | RichTextNode::ListItem(children) => {
                children.iter().map(|n| n.to_irc_formatted()).collect()
            }
        }
    }

    fn to_whatsapp_formatted(&self) -> String {
        match self {
            RichTextNode::Plain(s) => s.clone(),
            RichTextNode::Bold(children) => {
                format!(
                    "*{}*",
                    children
                        .iter()
                        .map(|n| n.to_whatsapp_formatted())
                        .collect::<String>()
                )
            }
            RichTextNode::Italic(children) => {
                format!(
                    "_{}_",
                    children
                        .iter()
                        .map(|n| n.to_whatsapp_formatted())
                        .collect::<String>()
                )
            }
            RichTextNode::Strikethrough(children) => {
                format!(
                    "~{}~",
                    children
                        .iter()
                        .map(|n| n.to_whatsapp_formatted())
                        .collect::<String>()
                )
            }
            RichTextNode::Code(s) => format!("`{}`", s),
            RichTextNode::CodeBlock { code, .. } => format!("```{}```", code),
            RichTextNode::Link { url, text } => {
                format!(
                    "{} ({})",
                    text.iter()
                        .map(|n| n.to_whatsapp_formatted())
                        .collect::<String>(),
                    url
                )
            }
            RichTextNode::Mention { name, .. } => format!("@{}", name),
            RichTextNode::Emoji(e) => e.clone(),
            RichTextNode::Paragraph(children) | RichTextNode::ListItem(children) => {
                children.iter().map(|n| n.to_whatsapp_formatted()).collect()
            }
        }
    }
}

fn html_escape(s: impl AsRef<str>) -> String {
    let s = s.as_ref();
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

impl RichText {
    /// Convert to plain text (strips all formatting).
    pub fn to_plain_text(&self) -> String {
        self.0.iter().map(|n| n.to_plain_text()).collect()
    }

    /// Convert to standard CommonMark markdown.
    pub fn to_markdown(&self) -> String {
        self.0.iter().map(|n| n.to_markdown()).collect()
    }

    /// Convert to Discord markdown (same as CommonMark).
    pub fn to_discord_markdown(&self) -> String {
        self.to_markdown()
    }

    /// Convert to Telegram HTML.
    pub fn to_telegram_html(&self) -> String {
        crate::markdown::markdown_to_telegram_html(self.to_markdown())
    }

    /// Convert to Slack mrkdwn.
    pub fn to_slack_mrkdwn(&self) -> String {
        crate::markdown::markdown_to_slack(self.to_markdown())
    }

    /// Convert to Matrix HTML.
    pub fn to_matrix_html(&self) -> String {
        self.0.iter().map(|n| n.to_matrix_html()).collect()
    }

    /// Convert to IRC formatted text (bold=`\x02`, italic=`\x1D`).
    pub fn to_irc_formatted(&self) -> String {
        self.0.iter().map(|n| n.to_irc_formatted()).collect()
    }

    /// Convert to WhatsApp formatted text.
    pub fn to_whatsapp_formatted(&self) -> String {
        self.0.iter().map(|n| n.to_whatsapp_formatted()).collect()
    }

    /// Create from plain text.
    pub fn from_plain(text: impl AsRef<str>) -> Self {
        Self(vec![RichTextNode::Plain(text.as_ref().to_string())])
    }

    /// Parse from Markdown using pulldown-cmark.
    pub fn from_markdown(text: impl AsRef<str>) -> Self {
        let text = text.as_ref();
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_STRIKETHROUGH);
        let parser = Parser::new_ext(text, opts);

        let mut stack: Vec<Vec<RichTextNode>> = vec![vec![]];

        for event in parser {
            match event {
                Event::Start(Tag::Strong)
                | Event::Start(Tag::Emphasis)
                | Event::Start(Tag::Strikethrough) => {
                    stack.push(vec![]);
                }
                Event::Start(Tag::Link { dest_url, .. }) => {
                    stack.push(vec![RichTextNode::Plain(dest_url.to_string())]);
                    stack.push(vec![]);
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    let lang = match kind {
                        pulldown_cmark::CodeBlockKind::Fenced(lang) if !lang.is_empty() => {
                            Some(lang.to_string())
                        }
                        _ => None,
                    };
                    stack.push(vec![RichTextNode::Plain(lang.unwrap_or_default())]);
                    stack.push(vec![]);
                }
                Event::End(TagEnd::Strong) => {
                    let children = stack.pop().unwrap_or_default();
                    if let Some(top) = stack.last_mut() {
                        top.push(RichTextNode::Bold(children));
                    }
                }
                Event::End(TagEnd::Emphasis) => {
                    let children = stack.pop().unwrap_or_default();
                    if let Some(top) = stack.last_mut() {
                        top.push(RichTextNode::Italic(children));
                    }
                }
                Event::End(TagEnd::Strikethrough) => {
                    let children = stack.pop().unwrap_or_default();
                    if let Some(top) = stack.last_mut() {
                        top.push(RichTextNode::Strikethrough(children));
                    }
                }
                Event::End(TagEnd::Link) => {
                    let link_text = stack.pop().unwrap_or_default();
                    let url_node = stack.pop().unwrap_or_default();
                    let url = if let Some(RichTextNode::Plain(u)) = url_node.into_iter().next() {
                        u
                    } else {
                        String::new()
                    };
                    if let Some(top) = stack.last_mut() {
                        top.push(RichTextNode::Link {
                            url,
                            text: link_text,
                        });
                    }
                }
                Event::End(TagEnd::CodeBlock) => {
                    let code_nodes = stack.pop().unwrap_or_default();
                    let lang_node = stack.pop().unwrap_or_default();
                    let lang = if let Some(RichTextNode::Plain(l)) = lang_node.into_iter().next() {
                        if l.is_empty() {
                            None
                        } else {
                            Some(l)
                        }
                    } else {
                        None
                    };
                    let code: String = code_nodes.iter().map(|n| n.to_plain_text()).collect();
                    if let Some(top) = stack.last_mut() {
                        top.push(RichTextNode::CodeBlock {
                            language: lang,
                            code,
                        });
                    }
                }
                Event::Code(text) => {
                    if let Some(top) = stack.last_mut() {
                        top.push(RichTextNode::Code(text.to_string()));
                    }
                }
                Event::Text(text) => {
                    if let Some(top) = stack.last_mut() {
                        top.push(RichTextNode::Plain(text.to_string()));
                    }
                }
                Event::SoftBreak | Event::HardBreak => {
                    if let Some(top) = stack.last_mut() {
                        top.push(RichTextNode::Plain("\n".into()));
                    }
                }
                _ => {}
            }
        }

        Self(stack.into_iter().next().unwrap_or_default())
    }
}

impl std::fmt::Display for RichText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_plain_text())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_strips_formatting() {
        let rt = RichText(vec![
            RichTextNode::Bold(vec![RichTextNode::Plain("hello".into())]),
            RichTextNode::Plain(" world".into()),
        ]);
        assert_eq!(rt.to_plain_text(), "hello world");
    }

    #[test]
    fn discord_bold_renders_stars() {
        let rt = RichText(vec![RichTextNode::Bold(vec![RichTextNode::Plain(
            "hi".into(),
        )])]);
        assert!(rt.to_discord_markdown().contains("**hi**"));
    }

    #[test]
    fn matrix_bold_renders_b_tag() {
        let rt = RichText(vec![RichTextNode::Bold(vec![RichTextNode::Plain(
            "hi".into(),
        )])]);
        assert!(rt.to_matrix_html().contains("<b>hi</b>"));
    }

    #[test]
    fn irc_bold_uses_control_char() {
        let rt = RichText(vec![RichTextNode::Bold(vec![RichTextNode::Plain(
            "hi".into(),
        )])]);
        let s = rt.to_irc_formatted();
        assert!(s.contains('\x02'));
    }

    #[test]
    fn whatsapp_bold_uses_stars() {
        let rt = RichText(vec![RichTextNode::Bold(vec![RichTextNode::Plain(
            "hi".into(),
        )])]);
        assert!(rt.to_whatsapp_formatted().contains("*hi*"));
    }

    #[test]
    fn from_markdown_parses_bold() {
        let rt = RichText::from_markdown("**bold text**");
        assert!(rt.0.iter().any(|n| matches!(n, RichTextNode::Bold(_))));
    }

    #[test]
    fn display_gives_plain_text() {
        let rt = RichText(vec![RichTextNode::Plain("hello".into())]);
        assert_eq!(rt.to_string(), "hello");
    }

    #[test]
    fn code_block_roundtrip() {
        let rt = RichText(vec![RichTextNode::CodeBlock {
            language: Some("rust".into()),
            code: "let x = 1;".into(),
        }]);
        let md = rt.to_markdown();
        assert!(md.contains("```rust"));
        assert!(md.contains("let x = 1;"));
    }
}
