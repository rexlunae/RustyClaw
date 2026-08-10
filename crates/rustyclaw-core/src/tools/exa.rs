//! Exa as a `web_search` provider alongside Brave.
//!
//! Exa (exa.ai) does semantic rather than keyword search, and returns page
//! text with each result instead of a snippet. It costs more per query and is
//! better for research questions where the right page does not contain the
//! words you searched for (issue #140).
//!
//! # Where these shapes come from
//!
//! `api.exa.ai` is unreachable from this environment, so none of this was
//! confirmed against a live call. Rather than write the request from memory —
//! which is how issue #411 happened, a client posting to endpoints that did
//! not exist and reporting success — the endpoint, the auth header, the
//! request fields and the response fields below are read out of Exa's own
//! published client, `exa-js` 2.17.0 on npm.
//!
//! Two things that differ from what issue #140 describes, both taken from the
//! SDK rather than the issue:
//!
//! * `category` does not accept "research paper" or "tweet". The current set
//!   is company, publication, news, personal site, financial report, people.
//! * `type` has three more values than the issue lists: hybrid, fast, instant.

use crate::tools::error::{ToolError, ToolResult};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;
use tracing::{debug, warn};

/// `POST` here with an `x-api-key` header. From `exa-js`: the client's default
/// `baseURL` is `https://api.exa.ai` and `search()` posts to `/search`.
const SEARCH_URL: &str = "https://api.exa.ai/search";

/// Search modes the API accepts.
///
/// An allowlist because a rejected mode comes back as an opaque 400: better to
/// say which value was wrong here than to relay the server's complaint.
const SEARCH_TYPES: &[&str] = &["auto", "neural", "keyword", "hybrid", "fast", "instant"];

/// Categories the API accepts, from `BaseSearchOptions["category"]`.
const CATEGORIES: &[&str] = &[
    "company",
    "publication",
    "news",
    "personal site",
    "financial report",
    "people",
];

/// Per-result page text. Matching `exa-js`'s own default rather than picking a
/// number: it sends `{ text: { maxCharacters: 10_000 } }`.
const MAX_CHARACTERS: u64 = 10_000;

/// One result. Only the fields this renders — the API sends more.
///
/// `title` is `string | null` in the SDK's `SearchResult`, not merely absent,
/// so it is an `Option` that also has to survive an explicit `null`.
#[derive(Debug, Deserialize)]
struct ExaResult {
    #[serde(default)]
    title: Option<String>,
    url: String,
    #[serde(rename = "publishedDate", default)]
    published_date: Option<String>,
    #[serde(default)]
    author: Option<String>,
    /// Present when `contents.text` was requested, which it always is here.
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExaResponse {
    #[serde(default)]
    results: Vec<ExaResult>,
    /// Exa rewrites the query when `useAutoprompt` is on, and says what it
    /// used. Worth showing: it explains results that look unrelated to what
    /// was typed.
    #[serde(rename = "autopromptString", default)]
    autoprompt_string: Option<String>,
}

/// Build the request body.
///
/// A free function over the parameters so the body can be asserted in a test
/// without a live API — which matters more than usual here, since the shape
/// was read from a published client rather than confirmed against the server.
pub(crate) fn search_body(
    query: &str,
    count: usize,
    search_type: Option<&str>,
    category: Option<&str>,
    use_autoprompt: Option<bool>,
) -> Value {
    let mut body = json!({
        "query": query,
        "numResults": count,
        // Always ask for text. Without it a result is a title and a URL, which
        // is what Brave already gives more cheaply — the page text is the
        // reason to pay for Exa.
        "contents": { "text": { "maxCharacters": MAX_CHARACTERS } },
    });

    let map = body.as_object_mut().expect("json! built an object");
    if let Some(t) = search_type {
        map.insert("type".to_string(), json!(t));
    }
    if let Some(c) = category {
        map.insert("category".to_string(), json!(c));
    }
    if let Some(a) = use_autoprompt {
        map.insert("useAutoprompt".to_string(), json!(a));
    }
    body
}

/// Check an enum-valued argument, naming the accepted values on refusal.
pub(crate) fn one_of<'a>(
    args: &'a Value,
    key: &str,
    allowed: &[&str],
) -> Result<Option<&'a str>, String> {
    let Some(raw) = args.get(key).and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    if !allowed.contains(&raw) {
        return Err(format!(
            "Unsupported {key} `{raw}`. Exa accepts: {}",
            allowed.join(", ")
        ));
    }
    Ok(Some(raw))
}

/// Render results for the model.
///
/// Separate from the request so the formatting — the part the model actually
/// reads — is testable against a recorded body.
pub(crate) fn render(query: &str, parsed: &ExaRendered) -> String {
    let mut out = String::new();
    out.push_str(&format!("Search results for: {query}\n"));
    if let Some(rewritten) = &parsed.autoprompt {
        // Not decoration: when this differs from the query, it is the reason
        // the results look the way they do.
        out.push_str(&format!("Exa searched for: {rewritten}\n"));
    }
    out.push('\n');

    for (i, r) in parsed.results.iter().enumerate() {
        out.push_str(&format!(
            "{}. {}\n",
            i + 1,
            r.title.as_deref().unwrap_or("(no title)")
        ));
        out.push_str(&format!("   {}\n", r.url));

        let mut meta = Vec::new();
        if let Some(author) = r.author.as_deref().filter(|a| !a.is_empty()) {
            meta.push(author.to_string());
        }
        if let Some(date) = r.published_date.as_deref().filter(|d| !d.is_empty()) {
            meta.push(date.to_string());
        }
        if !meta.is_empty() {
            out.push_str(&format!("   {}\n", meta.join(" · ")));
        }

        if let Some(text) = r.text.as_deref().filter(|t| !t.trim().is_empty()) {
            // The whole page would swamp the context — several thousand
            // characters per result, times ten results. web_fetch is the tool
            // for reading one of them in full.
            let excerpt: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let excerpt: String = excerpt.chars().take(500).collect();
            out.push_str(&format!("   {excerpt}\n"));
        }
        out.push('\n');
    }
    out
}

/// The parts of a response that [`render`] needs.
#[derive(Debug, Default)]
pub(crate) struct ExaRendered {
    pub results: Vec<RenderedResult>,
    pub autoprompt: Option<String>,
}

/// One result, flattened for rendering.
#[derive(Debug)]
pub(crate) struct RenderedResult {
    pub title: Option<String>,
    pub url: String,
    pub published_date: Option<String>,
    pub author: Option<String>,
    pub text: Option<String>,
}

/// Decode a reply body into the shape [`render`] takes.
pub(crate) fn decode(body: &str) -> Result<ExaRendered, String> {
    let raw: ExaResponse = serde_json::from_str(body)
        .map_err(|e| format!("Exa returned a body this client could not read: {e}"))?;
    Ok(ExaRendered {
        results: raw
            .results
            .into_iter()
            .map(|r| RenderedResult {
                title: r.title,
                url: r.url,
                published_date: r.published_date,
                author: r.author,
                text: r.text,
            })
            .collect(),
        autoprompt: raw.autoprompt_string,
    })
}

/// Run a search against Exa.
pub(crate) async fn search(args: &Value, query: &str, count: usize) -> ToolResult {
    let search_type = one_of(args, "search_type", SEARCH_TYPES)?;
    let category = one_of(args, "category", CATEGORIES)?;
    let use_autoprompt = args.get("use_autoprompt").and_then(|v| v.as_bool());

    let api_key = std::env::var("EXA_API_KEY").map_err(|_| {
        warn!("EXA_API_KEY environment variable not set");
        "EXA_API_KEY environment variable not set. Get a key at https://exa.ai/".to_string()
    })?;

    // Exa's numResults goes to 25; Brave's count stops at 10. The caller
    // clamps per provider, so this only guards against a caller that forgot.
    let count = count.clamp(1, 25);
    let body = search_body(query, count, search_type, category, use_autoprompt);

    debug!(count, ?search_type, ?category, "Searching with Exa API");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| ToolError::context("Failed to create HTTP client", e))?;

    let response = client
        .post(SEARCH_URL)
        .header("x-api-key", &api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ToolError::context("Exa Search request failed", e))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| ToolError::context("Failed to read the Exa response", e))?;

    if !status.is_success() {
        warn!(status = status.as_u16(), "Exa Search API error");
        return Err(format!(
            "Exa Search API error {}: {}",
            status.as_u16(),
            text.chars().take(400).collect::<String>()
        )
        .into());
    }

    let parsed = decode(&text)?;
    if parsed.results.is_empty() {
        debug!("No results found");
        return Ok("No results found.".to_string());
    }

    debug!(result_count = parsed.results.len(), "Search complete");
    Ok(render(query, &parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_always_asks_for_page_text() {
        let body = search_body("rust async", 5, None, None, None);
        assert_eq!(body["query"], "rust async");
        assert_eq!(body["numResults"], 5);
        // Without text, an Exa result is a title and a URL — which is what
        // Brave gives, more cheaply.
        assert_eq!(body["contents"]["text"]["maxCharacters"], 10_000);
    }

    /// Optional fields are omitted rather than sent as null, so the server
    /// applies its own defaults instead of being told "no category".
    #[test]
    fn unset_options_are_absent_from_the_body() {
        let body = search_body("q", 5, None, None, None);
        let map = body.as_object().unwrap();
        assert!(!map.contains_key("type"), "{body}");
        assert!(!map.contains_key("category"), "{body}");
        assert!(!map.contains_key("useAutoprompt"), "{body}");
    }

    /// And the wire names are Exa's, not this crate's.
    #[test]
    fn set_options_use_the_field_names_the_api_expects() {
        let body = search_body("q", 5, Some("neural"), Some("news"), Some(true));
        assert_eq!(body["type"], "neural");
        assert_eq!(body["category"], "news");
        assert_eq!(body["useAutoprompt"], true, "camelCase, not use_autoprompt");
    }

    /// Issue #140 lists categories ("research paper", "tweet") the current API
    /// does not accept. Following the issue would have produced a 400 on a
    /// value that reads perfectly reasonable.
    #[test]
    fn categories_the_api_does_not_accept_are_refused_locally() {
        let args = json!({"category": "research paper"});
        let err = one_of(&args, "category", CATEGORIES).expect_err("must be refused");
        assert!(err.contains("research paper"), "got: {err}");
        assert!(
            err.contains("publication"),
            "the message should list what works: {err}"
        );

        assert_eq!(
            one_of(&json!({"category": "news"}), "category", CATEGORIES).unwrap(),
            Some("news")
        );
    }

    #[test]
    fn search_types_beyond_the_issues_three_are_accepted() {
        for t in ["auto", "neural", "keyword", "hybrid", "fast", "instant"] {
            assert_eq!(
                one_of(&json!({ "search_type": t }), "search_type", SEARCH_TYPES).unwrap(),
                Some(t),
                "{t} is in the SDK's type union"
            );
        }
    }

    /// `title` is `string | null` in the SDK, not merely optional — an
    /// explicit null must decode rather than fail the whole response.
    #[test]
    fn a_null_title_decodes_and_renders() {
        let body = r#"{"results":[{"title":null,"url":"https://example.test/a","id":"x"}]}"#;
        let parsed = decode(body).expect("null title is valid");
        let out = render("q", &parsed);
        assert!(out.contains("(no title)"), "got: {out}");
        assert!(out.contains("https://example.test/a"), "got: {out}");
    }

    /// The keys were previously normalised with a whole-body string replace,
    /// which also rewrote those literals *inside* result text. Exa returns
    /// full page content, so searching for anything that discusses
    /// `publishedDate` — API docs, JSON schema pages — came back with the
    /// page silently altered. serde renames touch field names only.
    #[test]
    fn page_text_that_mentions_a_wire_key_is_not_rewritten() {
        let body = r#"{"results":[{"title":"T","url":"https://e.test/","id":"x",
            "text":"The API returns a publishedDate field on every record."}]}"#;
        let parsed = decode(body).expect("decodes");
        let out = render("exa api fields", &parsed);
        assert!(
            out.contains("publishedDate field"),
            "the page text was rewritten: {out}"
        );
        assert!(!out.contains("published_date"), "got: {out}");
    }

    /// The two camelCase keys this client reads have to survive.
    #[test]
    fn published_date_and_autoprompt_survive_the_wire_names() {
        let body = r#"{
            "autopromptString": "papers about rust async runtimes",
            "results": [{"title":"T","url":"https://e.test/","id":"x",
                         "publishedDate":"2026-01-02","author":"A","text":"body text"}]
        }"#;
        let parsed = decode(body).expect("decodes");
        assert_eq!(
            parsed.autoprompt.as_deref(),
            Some("papers about rust async runtimes")
        );

        let out = render("rust async", &parsed);
        assert!(
            out.contains("Exa searched for: papers about rust async runtimes"),
            "got: {out}"
        );
        assert!(out.contains("A · 2026-01-02"), "got: {out}");
    }

    /// Ten results at ten thousand characters each would swamp the context.
    #[test]
    fn page_text_is_excerpted_not_dumped() {
        let long = "word ".repeat(4000);
        let body = format!(
            r#"{{"results":[{{"title":"T","url":"https://e.test/","id":"x","text":"{}"}}]}}"#,
            long.trim()
        );
        let parsed = decode(&body).expect("decodes");
        let out = render("q", &parsed);
        assert!(
            out.len() < 1_000,
            "one result rendered {} characters",
            out.len()
        );
    }

    /// A body that is not the documented shape is an error, not an empty
    /// result set — "no results" and "I could not read the reply" are
    /// different answers and the caller acts differently on them.
    #[test]
    fn an_unreadable_body_is_an_error_rather_than_no_results() {
        let err = decode("<!DOCTYPE html><html>").expect_err("HTML is not a result set");
        assert!(err.contains("could not read"), "got: {err}");
    }
}
