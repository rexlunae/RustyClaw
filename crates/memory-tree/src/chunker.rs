//! Bounded markdown chunker with content-addressed IDs.
//!
//! Splits a long markdown document into chunks of <= `max_chars` characters,
//! preferring to break at paragraph boundaries, then sentences, then words.
//! Each chunk gets a deterministic SHA-256 id so re-ingesting the same input
//! is idempotent.

use sha2::{Digest, Sha256};

/// Approximate tokens-per-char ratio for English-ish markdown. The Memory
/// Tree budget is "≤3k tokens" but we don't have a real tokenizer here.
/// Empirically 4 chars/token is a fine default for the ≤3k bound.
pub const DEFAULT_MAX_CHARS: usize = 3_000 * 4;

/// One chunk of a canonicalized document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// SHA-256 of `source_id || \n || content`. 64 hex chars.
    pub id: String,
    /// Stable identifier for the source document (e.g. message id, file path).
    pub source_id: String,
    /// Zero-based index of this chunk within its source.
    pub index: usize,
    /// The chunk body.
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct ChunkerOptions {
    pub max_chars: usize,
    /// Refuse chunks smaller than this; merge into adjacent if possible.
    /// Default 100.
    pub min_chars: usize,
}

impl Default for ChunkerOptions {
    fn default() -> Self {
        Self {
            max_chars: DEFAULT_MAX_CHARS,
            min_chars: 100,
        }
    }
}

/// Split a markdown document into chunks. The first overload uses defaults;
/// pass [`ChunkerOptions`] to tune.
pub fn chunk(source_id: &str, content: &str) -> Vec<Chunk> {
    chunk_with(source_id, content, &ChunkerOptions::default())
}

pub fn chunk_with(source_id: &str, content: &str, opts: &ChunkerOptions) -> Vec<Chunk> {
    if content.is_empty() {
        return Vec::new();
    }
    if content.chars().count() <= opts.max_chars {
        let id = compute_id(source_id, content);
        return vec![Chunk {
            id,
            source_id: source_id.to_string(),
            index: 0,
            content: content.to_string(),
        }];
    }

    // Paragraph split first, then re-pack into <= max_chars buckets without
    // splitting paragraphs unless one is itself too large.
    let paragraphs = split_paragraphs(content);
    let mut buckets: Vec<String> = Vec::new();
    let mut cur = String::new();

    for para in paragraphs {
        if cur.chars().count() + para.chars().count() + 2 <= opts.max_chars {
            if !cur.is_empty() {
                cur.push_str("\n\n");
            }
            cur.push_str(&para);
        } else {
            if !cur.is_empty() {
                buckets.push(std::mem::take(&mut cur));
            }
            if para.chars().count() <= opts.max_chars {
                cur.push_str(&para);
            } else {
                // Paragraph too large; split by sentence, then by word/grapheme.
                for piece in split_oversized(&para, opts.max_chars) {
                    if !cur.is_empty() {
                        buckets.push(std::mem::take(&mut cur));
                    }
                    cur = piece;
                }
            }
        }
    }
    if !cur.is_empty() {
        buckets.push(cur);
    }

    // Merge any too-small trailing bucket into the previous one.
    if buckets.len() >= 2 {
        let last = buckets.last().cloned().unwrap_or_default();
        if last.chars().count() < opts.min_chars {
            let popped = buckets.pop().unwrap();
            if let Some(prev) = buckets.last_mut() {
                prev.push_str("\n\n");
                prev.push_str(&popped);
            }
        }
    }

    buckets
        .into_iter()
        .enumerate()
        .map(|(i, body)| Chunk {
            id: compute_id(&format!("{}#{}", source_id, i), &body),
            source_id: source_id.to_string(),
            index: i,
            content: body,
        })
        .collect()
}

fn split_paragraphs(s: &str) -> Vec<String> {
    s.split("\n\n")
        .map(|p| p.trim_matches('\n').to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

fn split_oversized(s: &str, max_chars: usize) -> Vec<String> {
    // Try sentence split first.
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut buf = String::new();

    let bytes = s.as_bytes();
    let mut i = 0;
    let mut last_end = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        buf.push(ch as char);
        if matches!(ch, b'.' | b'!' | b'?')
            && (i + 1 >= bytes.len() || bytes[i + 1].is_ascii_whitespace())
        {
            // End of a sentence.
            let sentence = &s[last_end..=i];
            if cur.chars().count() + sentence.chars().count() <= max_chars {
                cur.push_str(sentence);
            } else {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                if sentence.chars().count() <= max_chars {
                    cur.push_str(sentence);
                } else {
                    // Sentence itself too large — hard-split by char count.
                    out.extend(hard_split(sentence, max_chars));
                }
            }
            last_end = i + 1;
        }
        i += 1;
    }
    let tail = &s[last_end..];
    if !tail.is_empty() {
        if cur.chars().count() + tail.chars().count() <= max_chars {
            cur.push_str(tail);
        } else {
            out.push(std::mem::take(&mut cur));
            if tail.chars().count() <= max_chars {
                cur.push_str(tail);
            } else {
                out.extend(hard_split(tail, max_chars));
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn hard_split(s: &str, max_chars: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if cur.chars().count() >= max_chars {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn compute_id(source_id: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{:02x}", b);
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_doc_returns_single_chunk() {
        let chunks = chunk("doc-1", "Hello world");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[0].content, "Hello world");
        assert_eq!(chunks[0].id.len(), 64);
    }

    #[test]
    fn ids_are_deterministic() {
        let a = chunk("doc-1", "Hello world")[0].id.clone();
        let b = chunk("doc-1", "Hello world")[0].id.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn different_source_id_yields_different_chunk_id() {
        let a = chunk("doc-1", "Hello world")[0].id.clone();
        let b = chunk("doc-2", "Hello world")[0].id.clone();
        assert_ne!(a, b);
    }

    #[test]
    fn splits_by_paragraph_when_exceeding_max() {
        let opts = ChunkerOptions {
            max_chars: 80,
            min_chars: 0,
        };
        let doc = "First paragraph of moderate length.\n\nSecond paragraph also of moderate length.\n\nThird short.";
        let chunks = chunk_with("d", doc, &opts);
        assert!(chunks.len() >= 2);
        assert!(
            chunks
                .iter()
                .all(|c| c.content.chars().count() <= opts.max_chars + 10)
        );
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.index, i);
        }
    }

    #[test]
    fn hard_splits_a_single_giant_paragraph() {
        let opts = ChunkerOptions {
            max_chars: 50,
            min_chars: 0,
        };
        let long = "a".repeat(500);
        let chunks = chunk_with("d", &long, &opts);
        assert!(chunks.len() >= 10);
        for c in &chunks {
            assert!(c.content.chars().count() <= opts.max_chars);
        }
    }
}
