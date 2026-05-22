//! Apply transforms / filters / summarization to raw text.

use crate::compile::CompiledRule;

/// Apply a compiled rule to raw output, returning the reduced text and the
/// list of named counter values produced along the way.
pub fn reduce(rule: &CompiledRule, raw: &str) -> ReduceOutput {
    let mut text = raw.to_string();
    let t = &rule.layered.rule.transforms;

    if t.strip_ansi {
        text = strip_ansi(&text);
    }
    if t.pretty_print_json {
        text = pretty_print_json_lines(&text);
    }

    // Filter line-by-line.
    let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();

    if !rule.skip_patterns.is_empty() {
        lines.retain(|l| !rule.skip_patterns.iter().any(|p| p.is_match(l)));
    }
    if !rule.keep_patterns.is_empty() {
        lines.retain(|l| rule.keep_patterns.iter().any(|p| p.is_match(l)));
    }

    if t.dedupe_adjacent {
        lines.dedup();
    }
    if t.fold_blank_runs {
        let mut prev_blank = false;
        lines.retain(|l| {
            let blank = l.trim().is_empty();
            if blank && prev_blank {
                false
            } else {
                prev_blank = blank;
                true
            }
        });
    }
    if t.trim_empty_edges {
        while lines.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
            lines.remove(0);
        }
        while lines.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
            lines.pop();
        }
    }

    // Counters operate on the post-filter line set.
    let mut counters: Vec<NamedCount> = Vec::with_capacity(rule.counters.len());
    for c in &rule.counters {
        let mut n: usize = 0;
        for l in &lines {
            if c.pattern.is_match(l) {
                n += 1;
            }
        }
        counters.push(NamedCount {
            name: c.name.clone(),
            count: n,
        });
    }

    // Summarize.
    let summarized = if let Some(s) = &rule.layered.rule.summarize {
        let head = s.head.unwrap_or(0);
        let tail = s.tail.unwrap_or(0);
        if head == 0 && tail == 0 {
            lines
        } else if head + tail >= lines.len() {
            lines
        } else {
            let elided = lines.len() - head - tail;
            let mut out: Vec<String> = Vec::with_capacity(head + tail + 1);
            out.extend(lines.iter().take(head).cloned());
            out.push(format!("… [{} lines elided]", elided));
            out.extend(lines.iter().skip(lines.len() - tail).cloned());
            out
        }
    } else {
        lines
    };

    ReduceOutput {
        text: summarized.join("\n"),
        counters,
    }
}

#[derive(Debug)]
pub struct NamedCount {
    pub name: String,
    pub count: usize,
}

#[derive(Debug)]
pub struct ReduceOutput {
    pub text: String,
    pub counters: Vec<NamedCount>,
}

/// Strip ANSI CSI / OSC / SGR escape sequences. Conservative — matches the
/// common forms produced by terminal tooling without trying to be a full parser.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            // ESC. Skip until a terminator.
            i += 1;
            if i >= bytes.len() {
                break;
            }
            let next = bytes[i];
            i += 1;
            if next == b'[' {
                // CSI: parameters then a final byte in 0x40..=0x7e
                while i < bytes.len() {
                    let c = bytes[i];
                    i += 1;
                    if (0x40..=0x7e).contains(&c) {
                        break;
                    }
                }
            } else if next == b']' {
                // OSC: ends with BEL or ESC\
                while i < bytes.len() {
                    let c = bytes[i];
                    if c == 0x07 {
                        i += 1;
                        break;
                    }
                    if c == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            // Other ESC + single char: already consumed `next`, nothing else to do.
        } else {
            // Push one UTF-8 char by re-decoding from the original str.
            // (We track index into the str to keep boundaries valid.)
            let ch_start = i;
            // Advance past one codepoint.
            let s_rest = &s[ch_start..];
            let mut chars = s_rest.chars();
            if let Some(ch) = chars.next() {
                out.push(ch);
                i += ch.len_utf8();
            } else {
                break;
            }
        }
    }
    out
}

/// Pretty-print JSON object/array lines; pass other lines through untouched.
fn pretty_print_json_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut first = true;
    for line in s.lines() {
        if !first {
            out.push('\n');
        }
        first = false;

        let trimmed = line.trim();
        if (trimmed.starts_with('{') && trimmed.ends_with('}'))
            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
        {
            match serde_json::from_str::<serde_json::Value>(trimmed) {
                Ok(v) => {
                    if let Ok(pretty) = serde_json::to_string_pretty(&v) {
                        out.push_str(&pretty);
                        continue;
                    }
                }
                Err(_) => {}
            }
        }
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_basic() {
        let in_str = "\x1b[31mred\x1b[0m and \x1b[1mbold\x1b[0m";
        assert_eq!(strip_ansi(in_str), "red and bold");
    }

    #[test]
    fn strip_ansi_preserves_utf8() {
        let in_str = "\x1b[1m日本語\x1b[0m 🦀";
        assert_eq!(strip_ansi(in_str), "日本語 🦀");
    }
}
