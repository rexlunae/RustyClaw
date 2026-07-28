//! River — decentralized group chat on Freenet.
//!
//! A River room is one Freenet contract. Its state is a CBOR-encoded
//! [`ChatRoomStateV1`] holding the configuration, members, and a bounded window
//! of recent messages; its parameters are a CBOR-encoded [`ChatRoomParametersV1`]
//! naming the room owner. Reading a room is a contract GET plus a decode;
//! posting is a contract UPDATE carrying a signed message delta.
//!
//! The wire types come from `river-core`, River's own crate, so there is no
//! schema mirrored here to drift out of date.
//!
//! # Addressing a room
//!
//! Rooms are addressed by contract instance id — the value in a River invite
//! link, or shown by River's own UI. A room's id cannot be derived from its
//! owner key without the exact room-contract WASM (the id is a hash over code
//! *and* parameters), and that WASM is not distributed as a crate, so this
//! module takes the id directly rather than pretending it can compute one.
//!
//! # Private rooms
//!
//! River encrypts message bodies and nicknames in private rooms with a room
//! secret distributed to members out of band. This module has no access to
//! that secret, so private content is reported as encrypted rather than
//! rendered — never silently as blank text.

use std::time::{SystemTime, UNIX_EPOCH};

use river_core::room_state::member::MemberId;
use river_core::room_state::message::{AuthorizedMessageV1, MessageV1, RoomMessageBody};
use river_core::room_state::{ChatRoomParametersV1, ChatRoomStateV1, ChatRoomStateV1Delta};
use serde_json::json;

use super::client::{NodeClient, node_url, parse_instance_id};
use crate::tools::error::{ToolError, ToolResult};

/// Environment variable holding the `river:v1:sk:…` signing key used to post.
pub const SIGNING_KEY_ENV: &str = "RIVER_SIGNING_KEY";

/// Decode a room's contract state.
fn decode_room(bytes: &[u8]) -> ToolResult<ChatRoomStateV1> {
    if bytes.is_empty() {
        return Err(ToolError::from(
            "The contract returned no state. Either no peer is hosting this room, \
             or the id does not name a River room.",
        ));
    }
    ciborium::de::from_reader(bytes).map_err(|e| {
        ToolError::from(format!(
            "The contract state is not a River room ({} bytes did not decode: {e}).",
            bytes.len()
        ))
    })
}

/// Decode a room's contract parameters, which carry the owner's key.
fn decode_params(bytes: &[u8]) -> ToolResult<ChatRoomParametersV1> {
    ciborium::de::from_reader(bytes)
        .map_err(|e| ToolError::from(format!("Could not read the room's parameters: {e}")))
}

/// The nickname a member chose, when the room is public enough to show it.
///
/// Returns `None` for a private room; the caller renders the member id instead
/// of inventing a name.
///
/// Goes through `MemberInfoV1::canonical` rather than a first-match search: a
/// state can legitimately carry more than one record per member, and a bare
/// `find` can read the losing one.
fn nickname(state: &ChatRoomStateV1, id: &MemberId) -> Option<String> {
    let info = state.member_info.canonical(*id)?;
    let bytes = info.member_info.preferred_nickname.as_public_bytes()?;
    String::from_utf8(bytes.to_vec()).ok()
}

/// A member's display label: nickname if readable, else the member id.
fn label(state: &ChatRoomStateV1, id: &MemberId) -> String {
    nickname(state, id).unwrap_or_else(|| format!("{id}"))
}

/// Seconds since the Unix epoch, or 0 for the (impossible in practice) case of
/// a timestamp before it.
fn epoch_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Render one message. Encrypted bodies are labelled, not blanked.
fn message_json(state: &ChatRoomStateV1, msg: &AuthorizedMessageV1) -> serde_json::Value {
    let body = match &msg.message.content {
        RoomMessageBody::Public { .. } => msg
            .message
            .content
            .decode_content()
            .map(|c| c.to_display_string())
            .unwrap_or_else(|| "[unreadable message content]".to_string()),
        RoomMessageBody::Private { .. } => {
            "[encrypted — this is a private room and the room secret is not available here]"
                .to_string()
        }
    };
    json!({
        "id": msg.id().to_string(),
        "author": label(state, &msg.message.author),
        "author_id": msg.message.author.to_string(),
        "time": epoch_secs(msg.message.time),
        "encrypted": matches!(msg.message.content, RoomMessageBody::Private { .. }),
        "text": body,
    })
}

/// Fetch a room, optionally also its parameters.
async fn fetch_room(
    room: &str,
    url: Option<&str>,
    want_params: bool,
) -> ToolResult<(ChatRoomStateV1, Option<ChatRoomParametersV1>)> {
    let id = parse_instance_id(room)?;
    let url = node_url(url);
    let mut client = NodeClient::connect(&url).await?;
    let fetched = client.get(id, want_params).await?;
    let state = decode_room(&fetched.state)?;
    let params = match fetched.parameters {
        Some(bytes) => Some(decode_params(&bytes)?),
        None if want_params => {
            return Err(ToolError::from(
                "The node returned the room state without the contract, so the \
                 owner key needed to address a message is unavailable.",
            ));
        }
        None => None,
    };
    Ok((state, params))
}

/// `read` — the room's recent messages, oldest first.
pub async fn read(room: &str, url: Option<&str>, limit: usize) -> ToolResult {
    let (state, _) = fetch_room(room, url, false).await?;
    let all = &state.recent_messages.messages;
    // A room holds only a bounded window of recent messages; take the tail so
    // `limit` means "the most recent N", then present them in reading order.
    let start = all.len().saturating_sub(limit);
    let shown: Vec<_> = all[start..]
        .iter()
        .map(|m| message_json(&state, m))
        .collect();
    Ok(json!({
        "room": room,
        "retained": all.len(),
        "showing": shown.len(),
        "members": state.members.members.len(),
        "messages": shown,
    })
    .to_string())
}

/// `members` — who is in the room, and who invited them.
pub async fn members(room: &str, url: Option<&str>) -> ToolResult {
    let (state, params) = fetch_room(room, url, true).await?;
    let owner_id = params.as_ref().map(|p| p.owner_id());
    let list: Vec<_> = state
        .members
        .members
        .iter()
        .map(|m| {
            json!({
                "member_id": m.member.id().to_string(),
                "name": label(&state, &m.member.id()),
                "invited_by": m.member.invited_by.to_string(),
            })
        })
        .collect();
    Ok(json!({
        "room": room,
        "owner": owner_id.map(|id| id.to_string()),
        "count": list.len(),
        "members": list,
    })
    .to_string())
}

/// Resolve the signing key used to author a message.
///
/// The environment variable is the intended path: it keeps the key out of tool
/// arguments, which are logged and shown in transcripts. An explicit argument
/// is accepted for one-off use and documented as the lesser option.
fn signing_key(explicit: Option<&str>) -> ToolResult<ed25519_dalek::SigningKey> {
    let encoded = match explicit.filter(|s| !s.trim().is_empty()) {
        Some(s) => s.trim().to_string(),
        None => std::env::var(SIGNING_KEY_ENV)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                ToolError::from(format!(
                    "No River signing key available. Set {SIGNING_KEY_ENV} to your \
                     `river:v1:sk:…` key (preferred — it keeps the key out of tool \
                     arguments), or pass `signing_key`."
                ))
            })?,
    };
    match river_core::crypto_values::CryptoValue::from_encoded_string(&encoded) {
        Ok(river_core::crypto_values::CryptoValue::SigningKey(key)) => Ok(key),
        Ok(_) => Err(ToolError::from(
            "That is a River key, but not a signing key. Posting needs the \
             `river:v1:sk:…` form, not a verifying key or signature.",
        )),
        Err(e) => Err(ToolError::from(format!(
            "Could not read the River signing key: {e}"
        ))),
    }
}

/// `send` — post a public text message to a room.
///
/// The author must already be a member: the room contract rejects a message
/// from a key it does not know, and that rejection surfaces as a node error
/// rather than a silent no-op.
pub async fn send(room: &str, text: &str, key: Option<&str>, url: Option<&str>) -> ToolResult {
    if text.trim().is_empty() {
        return Err(ToolError::from("Refusing to post an empty message."));
    }
    let signing = signing_key(key)?;
    let id = parse_instance_id(room)?;
    let url = node_url(url);

    let mut client = NodeClient::connect(&url).await?;
    // The owner id is part of every message, so the room must be fetched with
    // its contract before anything can be addressed to it.
    let fetched = client.get(id, true).await?;
    let state = decode_room(&fetched.state)?;
    let missing_contract = || {
        ToolError::from(
            "The node returned the room state without the contract, so the owner \
             key and code hash needed to address a message are unavailable.",
        )
    };
    let params = fetched
        .parameters
        .ok_or_else(missing_contract)
        .and_then(|bytes| decode_params(&bytes))?;
    let contract_key = fetched.key.ok_or_else(missing_contract)?;

    let author = MemberId::from(&signing.verifying_key());
    let is_member = state
        .members
        .members
        .iter()
        .any(|m| m.member.id() == author);
    if !is_member && author != params.owner_id() {
        return Err(ToolError::from(format!(
            "This key is not a member of room {room} (member id {author}). \
             River rooms are invite-only; the contract would reject the message."
        )));
    }

    if state.configuration.configuration.privacy_mode
        != river_core::room_state::privacy::PrivacyMode::Public
    {
        return Err(ToolError::from(
            "This is a private room. Messages there must be encrypted with the \
             room secret, which is distributed to members out of band and is not \
             available here — posting plaintext would be rejected by the contract.",
        ));
    }

    let message = MessageV1 {
        room_owner: params.owner_id(),
        author,
        time: SystemTime::now(),
        content: RoomMessageBody::public(text.to_string()),
    };
    let authorized = AuthorizedMessageV1::new(message, &signing);
    let message_id = authorized.id().to_string();

    let delta = ChatRoomStateV1Delta {
        recent_messages: Some(vec![authorized]),
        ..Default::default()
    };
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&delta, &mut bytes)
        .map_err(|e| ToolError::from(format!("Could not encode the message delta: {e}")))?;

    client.update_delta(contract_key, bytes).await?;
    Ok(json!({
        "room": room,
        "sent": true,
        "message_id": message_id,
        "author": author.to_string(),
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_state_is_reported_as_missing_not_as_an_empty_room() {
        let err = decode_room(&[]).expect_err("empty state should be an error");
        assert!(err.to_string().contains("no peer is hosting"), "{err}");
    }

    #[test]
    fn undecodable_state_says_what_it_expected() {
        let err = decode_room(b"not cbor").expect_err("garbage should not decode");
        assert!(err.to_string().contains("not a River room"), "{err}");
    }

    #[test]
    fn a_room_round_trips_through_cbor() {
        let state = ChatRoomStateV1::default();
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&state, &mut buf).expect("encode");
        assert_eq!(decode_room(&buf).expect("decode"), state);
    }

    #[test]
    fn a_verifying_key_is_rejected_where_a_signing_key_is_required() {
        let vk = river_core::crypto_values::CryptoValue::VerifyingKey(
            ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]).verifying_key(),
        )
        .to_encoded_string();
        let err = signing_key(Some(&vk)).expect_err("a verifying key should not post");
        assert!(err.to_string().contains("not a signing key"), "{err}");
    }

    #[test]
    fn a_signing_key_round_trips_through_its_encoded_form() {
        let key = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let encoded =
            river_core::crypto_values::CryptoValue::SigningKey(key.clone()).to_encoded_string();
        let parsed = signing_key(Some(&encoded)).expect("should parse");
        assert_eq!(parsed.to_bytes(), key.to_bytes());
    }

    #[test]
    fn a_malformed_key_is_named_as_such() {
        let err = signing_key(Some("river:v1:sk:!!!")).expect_err("garbage should not parse");
        assert!(err.to_string().contains("Could not read"), "{err}");
    }

    #[test]
    fn a_missing_key_names_the_env_var() {
        if std::env::var(SIGNING_KEY_ENV).is_ok() {
            return;
        }
        let err = signing_key(None).expect_err("should require a key");
        assert!(err.to_string().contains(SIGNING_KEY_ENV), "{err}");
    }

    #[test]
    fn epoch_conversion_survives_a_pre_epoch_timestamp() {
        assert_eq!(epoch_secs(UNIX_EPOCH), 0);
        assert_eq!(
            epoch_secs(UNIX_EPOCH + std::time::Duration::from_secs(42)),
            42
        );
    }
}
