//! Fast scoring heuristics. Cheap, no LLM. Used to decide whether a chunk is
//! worth admitting to the tree at ingest time. A real implementation would
//! also factor entity density and recency; this one is intentionally simple.

use crate::chunker::Chunk;

/// Compute a 0.0..=1.0 admission score. Higher means "more likely worth
/// keeping". The defaults bias toward "keep most things" so users with
/// already-curated data aren't surprised by aggressive drops.
pub fn fast_score(chunk: &Chunk) -> f64 {
    let text = &chunk.content;
    if text.is_empty() {
        return 0.0;
    }

    let mut score: f64 = 0.5;

    // Bias on length: very short chunks (< 50 chars) are usually low-signal.
    let len = text.chars().count();
    if len < 50 {
        score -= 0.3;
    } else if len > 400 {
        score += 0.1;
    }

    // Reward for structure: bullet lists, headings, code blocks.
    if text
        .lines()
        .any(|l| l.starts_with("# ") || l.starts_with("## "))
    {
        score += 0.1;
    }
    if text.lines().any(|l| l.trim_start().starts_with("- ")) {
        score += 0.05;
    }
    if text.contains("```") {
        score += 0.05;
    }

    // Penalize obvious chrome / boilerplate.
    let lower = text.to_lowercase();
    for marker in [
        "unsubscribe",
        "this message was sent automatically",
        "do not reply",
    ] {
        if lower.contains(marker) {
            score -= 0.2;
        }
    }

    score.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::Chunk;

    fn mk(content: &str) -> Chunk {
        Chunk {
            id: "x".into(),
            source_id: "x".into(),
            index: 0,
            content: content.to_string(),
        }
    }

    #[test]
    fn structured_content_scores_higher_than_one_liner() {
        let a = fast_score(&mk(
            "# Heading\n\n- bullet one\n- bullet two\n\nA paragraph that goes on for a while with enough text to be interesting and worth keeping in the buffer.",
        ));
        let b = fast_score(&mk("hi"));
        assert!(a > b);
    }

    #[test]
    fn unsubscribe_chrome_penalized() {
        let a = fast_score(&mk(
            "Click unsubscribe to opt out. This message was sent automatically.",
        ));
        let b = fast_score(&mk(
            "A more normal paragraph about a meaningful topic that someone would want to remember.",
        ));
        assert!(a < b);
    }
}
