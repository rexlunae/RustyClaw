//! Atlas — the decentralized discovery layer for Freenet.
//!
//! An Atlas index is an ordinary Freenet contract whose state is a CBOR-encoded
//! map from subject id to a signed record. Reading it is therefore a plain
//! contract GET followed by a decode; searching is done here, client-side,
//! because the contract exposes no query interface.
//!
//! # Where these types come from
//!
//! Atlas ships its schema in an `atlas-common` crate that is **not published**
//! (the `atlas-common` on crates.io is an unrelated Intel Labs project), so the
//! wire types are mirrored here from `freenet/atlas`'s `common/src`. Only the
//! fields needed to decode and render an index are modelled — enough to read,
//! not to write.
//!
//! # What is not checked
//!
//! Records carry ed25519 signatures chaining to the index's root key. Those are
//! *not* re-verified here: the state arrives from a local node that already ran
//! the contract's `validate_state`, and re-verifying would need the contract
//! parameters (the root key) that a plain GET does not return. Treat entries as
//! being exactly as trustworthy as the index you pointed at.
//!
//! # Which index?
//!
//! Atlas is deliberately pluralistic — many competing indexes, no canonical
//! one — so there is no built-in default. The index contract id must be given
//! per call or via `$ATLAS_INDEX_CONTRACT`.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::client::{NodeClient, node_url, parse_instance_id};
use crate::tools::error::{ToolError, ToolResult};

/// Environment variable naming the index contract to search.
pub const INDEX_ENV: &str = "ATLAS_INDEX_CONTRACT";

/// Opaque, stable handle for one indexed subject (base58 over 9 random bytes).
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct SubjectId(String);

/// What an entry describes. Atlas 0.1 indexes only directly openable things.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    App,
    Site,
    External,
}

/// Where "open this entry" navigates.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum Locator {
    /// A resource on Freenet: a full contract instance id plus an opaque path.
    Freenet { contract_id: String, path: String },
    /// An external web resource (https only, per the Atlas schema).
    External { url: String },
}

impl Locator {
    /// Canonical string form, as Atlas's own clients render it.
    fn to_uri(&self) -> String {
        match self {
            Locator::Freenet { contract_id, path } => format!("freenet:{contract_id}{path}"),
            Locator::External { url } => url.clone(),
        }
    }
}

/// A self-contained result card: everything needed to show an entry without
/// fetching anything else.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct IndexEntry {
    pub subject_id: SubjectId,
    pub version: u64,
    pub kind: Kind,
    pub title: String,
    pub snippet: String,
    pub tags: Vec<String>,
    pub locator: Locator,
    pub featured: bool,
    /// Unix seconds, set by the curator (contracts cannot read a clock).
    pub added_at: u64,
}

/// Removal marker for a subject.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Tombstone {
    pub subject_id: SubjectId,
    pub version: u64,
}

/// The signable body of a record: either a live entry or its removal.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub enum RecordBody {
    Live(IndexEntry),
    Tomb(Tombstone),
}

/// A record signed by one of the index's authorized online keys.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct SignedRecord {
    pub body: RecordBody,
    pub by: VerifyingKey,
    pub sig: Signature,
}

/// Root-signed authorization of the online signing keys.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct KeyAuthBody {
    pub version: u64,
    pub authorized: Vec<VerifyingKey>,
}

/// [`KeyAuthBody`] plus the root key's signature over it.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct KeyAuth {
    pub body: KeyAuthBody,
    pub sig: Signature,
}

/// The full index state: the key authorization plus one current record per
/// subject.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
pub struct IndexState {
    pub key_auth: Option<KeyAuth>,
    pub records: BTreeMap<SubjectId, SignedRecord>,
}

impl IndexState {
    /// Entries that have not been tombstoned.
    pub fn live_entries(&self) -> impl Iterator<Item = &IndexEntry> {
        self.records.values().filter_map(|r| match &r.body {
            RecordBody::Live(e) => Some(e),
            RecordBody::Tomb(_) => None,
        })
    }
}

/// Decode index state bytes fetched from a node.
pub fn decode_index(bytes: &[u8]) -> ToolResult<IndexState> {
    if bytes.is_empty() {
        return Err(ToolError::from(
            "The contract returned no state. Either the index was never \
             initialized, or the id points at something that is not an Atlas index.",
        ));
    }
    ciborium::de::from_reader(bytes).map_err(|e| {
        ToolError::from(format!(
            "The contract state is not an Atlas index ({} bytes did not decode: {e}). \
             Check that the id names an index contract and not, say, Atlas's web UI \
             container.",
            bytes.len()
        ))
    })
}

/// Score an entry against a whitespace-separated query.
///
/// Every term must appear somewhere in the entry (an AND match); the score just
/// orders the survivors, weighting a title hit above a tag above the snippet so
/// that "chat" ranks a chat app over a page that merely mentions chat.
fn score(entry: &IndexEntry, terms: &[String]) -> Option<u32> {
    let title = entry.title.to_lowercase();
    let snippet = entry.snippet.to_lowercase();
    let tags: Vec<String> = entry.tags.iter().map(|t| t.to_lowercase()).collect();

    let mut total = 0;
    for term in terms {
        let mut best = 0;
        if title.contains(term) {
            best = best.max(if title == *term { 100 } else { 50 });
        }
        if tags.iter().any(|t| t == term) {
            best = best.max(30);
        } else if tags.iter().any(|t| t.contains(term)) {
            best = best.max(20);
        }
        if snippet.contains(term) {
            best = best.max(10);
        }
        if best == 0 {
            return None;
        }
        total += best;
    }
    if entry.featured {
        total += 5;
    }
    Some(total)
}

/// Render one entry as JSON for tool output.
fn entry_json(entry: &IndexEntry) -> serde_json::Value {
    json!({
        "subject_id": entry.subject_id.0,
        "kind": format!("{:?}", entry.kind),
        "title": entry.title,
        "snippet": entry.snippet,
        "tags": entry.tags,
        "open": entry.locator.to_uri(),
        "featured": entry.featured,
        "added_at": entry.added_at,
        "version": entry.version,
    })
}

/// Resolve the index contract id: explicit argument, then `$ATLAS_INDEX_CONTRACT`.
fn resolve_index(explicit: Option<&str>) -> ToolResult<String> {
    if let Some(id) = explicit.filter(|s| !s.trim().is_empty()) {
        return Ok(id.trim().to_string());
    }
    std::env::var(INDEX_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            ToolError::from(format!(
                "No Atlas index specified. Atlas has no single canonical index — \
                 pass `index` with the index contract's instance id, or set {INDEX_ENV}."
            ))
        })
}

/// Fetch and decode an index, shared by every action.
async fn fetch_index(index: Option<&str>, url: Option<&str>) -> ToolResult<(IndexState, String)> {
    let index = resolve_index(index)?;
    let id = parse_instance_id(&index)?;
    let url = node_url(url);
    let mut client = NodeClient::connect(&url).await?;
    let fetched = client.get(id, false).await?;
    Ok((decode_index(&fetched.state)?, index))
}

/// `search` — rank live entries against a query.
pub async fn search(
    query: &str,
    index: Option<&str>,
    url: Option<&str>,
    limit: usize,
) -> ToolResult {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_lowercase())
        .filter(|t| !t.is_empty())
        .collect();
    if terms.is_empty() {
        return Err(ToolError::from(
            "Empty query. Use the `list` action to browse an index without a query.",
        ));
    }

    let (state, index_id) = fetch_index(index, url).await?;
    let total = state.live_entries().count();

    let mut hits: Vec<(u32, &IndexEntry)> = state
        .live_entries()
        .filter_map(|e| score(e, &terms).map(|s| (s, e)))
        .collect();
    // Ties break on recency so a repeated search is stable and shows new things.
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.added_at.cmp(&a.1.added_at)));

    let shown = hits.len().min(limit);
    Ok(json!({
        "index": index_id,
        "query": query,
        "matched": hits.len(),
        "indexed": total,
        "showing": shown,
        "results": hits[..shown].iter().map(|(_, e)| entry_json(e)).collect::<Vec<_>>(),
    })
    .to_string())
}

/// `list` — browse an index newest-first, without a query.
pub async fn list(index: Option<&str>, url: Option<&str>, limit: usize) -> ToolResult {
    let (state, index_id) = fetch_index(index, url).await?;
    let mut entries: Vec<&IndexEntry> = state.live_entries().collect();
    entries.sort_by(|a, b| {
        b.featured
            .cmp(&a.featured)
            .then(b.added_at.cmp(&a.added_at))
    });
    let shown = entries.len().min(limit);
    Ok(json!({
        "index": index_id,
        "indexed": entries.len(),
        "showing": shown,
        "entries": entries[..shown].iter().map(|e| entry_json(e)).collect::<Vec<_>>(),
    })
    .to_string())
}

/// `get` — look one subject up by its id.
pub async fn get(subject: &str, index: Option<&str>, url: Option<&str>) -> ToolResult {
    let (state, index_id) = fetch_index(index, url).await?;
    let wanted = subject.trim();
    let found = state.live_entries().find(|e| e.subject_id.0 == wanted);
    match found {
        Some(entry) => Ok(json!({ "index": index_id, "entry": entry_json(entry) }).to_string()),
        None => {
            // A tombstoned subject is a different answer from an unknown one,
            // and saying so saves the caller a pointless retry.
            let removed = state.records.contains_key(&SubjectId(wanted.to_string()));
            Err(ToolError::from(if removed {
                format!("Subject '{wanted}' was removed from index {index_id}.")
            } else {
                format!("Subject '{wanted}' is not in index {index_id}.")
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(title: &str, snippet: &str, tags: &[&str]) -> IndexEntry {
        IndexEntry {
            subject_id: SubjectId("1111111111111".into()),
            version: 1,
            kind: Kind::App,
            title: title.into(),
            snippet: snippet.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            locator: Locator::External {
                url: "https://example.com".into(),
            },
            featured: false,
            added_at: 1,
        }
    }

    #[test]
    fn every_term_must_match() {
        let e = entry("River", "group chat", &["chat"]);
        assert!(score(&e, &["chat".into()]).is_some());
        assert!(score(&e, &["chat".into(), "spreadsheet".into()]).is_none());
    }

    #[test]
    fn a_title_hit_outranks_a_snippet_hit() {
        let in_title = entry("Chat app", "", &[]);
        let in_snippet = entry("Something", "a chat app", &[]);
        let term = vec!["chat".to_string()];
        assert!(score(&in_title, &term) > score(&in_snippet, &term));
    }

    #[test]
    fn an_exact_tag_outranks_a_partial_one() {
        let exact = entry("x", "", &["chat"]);
        let partial = entry("x", "", &["chatter"]);
        let term = vec!["chat".to_string()];
        assert!(score(&exact, &term) > score(&partial, &term));
    }

    #[test]
    fn empty_state_is_reported_as_uninitialized_not_as_an_empty_index() {
        let err = decode_index(&[]).expect_err("empty state should be an error");
        assert!(err.to_string().contains("never initialized"), "{err}");
    }

    #[test]
    fn undecodable_state_says_what_it_expected() {
        let err = decode_index(b"not cbor at all").expect_err("garbage should not decode");
        assert!(err.to_string().contains("not an Atlas index"), "{err}");
    }

    #[test]
    fn a_missing_index_id_names_the_env_var() {
        // Only exercised when the env var is unset; skip rather than mutate
        // process-global state under a parallel test runner.
        if std::env::var(INDEX_ENV).is_ok() {
            return;
        }
        let err = resolve_index(None).expect_err("should require an index");
        assert!(err.to_string().contains(INDEX_ENV), "{err}");
    }

    #[test]
    fn a_freenet_locator_renders_as_a_uri() {
        let loc = Locator::Freenet {
            contract_id: "abc".into(),
            path: "/index.html".into(),
        };
        assert_eq!(loc.to_uri(), "freenet:abc/index.html");
    }

    #[test]
    fn an_index_round_trips_through_cbor() {
        // Pins the shape this module decodes: whatever we can write, we read.
        let state = IndexState::default();
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&state, &mut buf).expect("encode");
        assert_eq!(decode_index(&buf).expect("decode"), state);
    }
}
