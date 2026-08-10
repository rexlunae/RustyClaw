//! Message relevance pre-filter — the rule tier of RustyClaw#165, plus the
//! LLM classifier tier (`RelevanceFilter::Smart`).
//!
//! Before a full processing cycle is spent on an incoming messenger
//! message, the gateway can decide whether the message is relevant at all.
//! With [`RelevanceFilter::Always`] (the default) everything is processed,
//! preserving historic behavior. With [`RelevanceFilter::Mentions`] a
//! group-chat message is processed only when it:
//!
//! * is a direct message (mirroring `resolve_routing`'s DM test), or
//! * mentions the agent by name (`@Name` or the name as a whole word), or
//! * replies to a message the agent sent in that channel.
//!
//! [`RelevanceFilter::Smart`] runs the same rules first, then asks a
//! classifier model call for messages the rules cannot prove relevant (see
//! [`classify_relevance`]). The classifier runs inside the processing task,
//! never the poll loop, so a model call cannot stall message polling.

use std::collections::{HashMap, VecDeque};

use rustyclaw_core::config::{Config, MessengerConfig, RelevanceFilter};
use rustyclaw_core::messengers::Message;
use tracing::warn;

/// Number of sent message IDs remembered per channel. Replies arrive shortly
/// after the message they target; a few hundred entries per channel is far
/// more than enough and keeps memory flat.
const TRACKER_CAP_PER_CHANNEL: usize = 512;

/// Message IDs this agent has sent, keyed by channel, so an incoming
/// `reply_to` can be recognized as a reply to the agent.
///
/// The gateway's only messenger send path records into this tracker (see
/// `process_incoming_message`), so every reply target the agent produced in
/// a conversation is covered. Messages sent by scheduled jobs or triggers
/// that do not go through the messenger handler are out of scope for the
/// rule tier; the classifier tier will not depend on this tracker.
#[derive(Debug)]
pub struct SentMessageTracker {
    by_channel: HashMap<String, VecDeque<String>>,
    cap: usize,
}

impl SentMessageTracker {
    /// Construct an empty tracker with the default per-channel cap
    /// ([`TRACKER_CAP_PER_CHANNEL`]).
    pub fn new() -> Self {
        Self {
            by_channel: HashMap::new(),
            cap: TRACKER_CAP_PER_CHANNEL,
        }
    }

    /// Record that the agent sent `message_id` in the channel keyed by
    /// `key` (see [`channel_key`]).
    pub fn record(&mut self, key: &str, message_id: &str) {
        let queue = self.by_channel.entry(key.to_string()).or_default();
        if queue.len() >= self.cap {
            queue.pop_front();
        }
        queue.push_back(message_id.to_string());
    }

    /// Constructor with an explicit per-channel cap, for tests that exercise
    /// the bound.
    #[cfg(test)]
    fn with_cap_for_test(cap: usize) -> Self {
        Self {
            by_channel: HashMap::new(),
            cap,
        }
    }

    /// Whether `message_id` is one the agent sent in the channel keyed by
    /// `key`.
    pub fn contains(&self, key: &str, message_id: &str) -> bool {
        self.by_channel
            .get(key)
            .is_some_and(|queue| queue.iter().any(|id| id == message_id))
    }
}

// Manual `Default` (not derived): the derived one would leave `cap` at 0,
// silently keeping at most one entry per channel. Default must behave like
// the real constructor.
impl Default for SentMessageTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable key for the sent-message tracker: account + channel, with DMs
/// folded into `dm` (DMs are always relevant anyway, so they are only ever
/// recorded defensively).
pub fn channel_key(account_name: &str, channel: Option<&str>) -> String {
    format!("{account_name}:{}", channel.unwrap_or("dm"))
}

/// The names a human would use to @-mention this agent on a given account:
/// the account's presented display name, the configured agent name, and the
/// platform handles the account actually answers to.  Some backends address
/// bots by handle rather than display name — IRC by nick, Matrix by the
/// user-id local part — so omitting those would make the agent unreachable
/// in group chats under `RelevanceFilter::Mentions`.
pub fn mention_tokens(config: &Config, account: &MessengerConfig) -> Vec<String> {
    let mut tokens = Vec::new();
    let profile = super::resolved_profile(account, config);
    let display = profile.display_name.trim();
    if !display.is_empty() {
        tokens.push(display.to_string());
    }
    let agent = config.agent_name.trim();
    if !agent.is_empty() {
        tokens.push(agent.to_string());
    }
    // IRC: the nick chosen at connect time. Prefer the explicitly configured
    // nick; otherwise the sanitized form `apply_profile` would derive from
    // the display name (non-alphanumerics → '_', 16 chars).
    match account
        .nick
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        Some(nick) => tokens.push(nick.to_string()),
        None if !display.is_empty() => {
            let derived: String = display
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .take(16)
                .collect();
            if !derived.is_empty() && derived != display {
                tokens.push(derived);
            }
        }
        None => {}
    }
    // Matrix: the local part of the user id ("@user:homeserver" → "user") is
    // what people type to address the bot before the homeserver suffix.
    if let Some(uid) = account
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    {
        let local = uid.trim_start_matches('@').split(':').next().unwrap_or("");
        if !local.is_empty() {
            tokens.push(local.to_string());
        }
    }
    tokens.sort();
    tokens.dedup();
    tokens
}

/// True when `content` mentions `name` — as `@name` or as a whole word,
/// case-insensitively. Whole-word matching keeps short names from matching
/// inside other words (e.g. "ada" inside "adapter").
pub fn contains_mention(content: &str, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() || content.is_empty() {
        return false;
    }
    let haystack = content.to_lowercase();
    let needle = name.to_lowercase();

    // "@name" — the most common way to address a bot. The character after the
    // name must not be alphanumeric, so "@adaline" is not "@ada" and
    // "bob@adaline.example" does not address "ada".
    if let Some(pos) = haystack.find(&format!("@{needle}")) {
        let after = &haystack[pos + 1 + needle.len()..];
        if after.chars().next().is_none_or(|c| !c.is_alphanumeric()) {
            return true;
        }
    }

    // Whole-word match: "name:" after a call-out, "name," mid-sentence, or
    // the bare name on its own. The character before and after the match
    // must not be alphanumeric. `offset` tracks the match position in the
    // *full* haystack: without it, a name repeated inside one word (e.g.
    // "AdaAda") would pass the `pos == 0` boundary check on the second
    // pass over the remainder.
    let mut rest = haystack.as_str();
    let mut offset = 0usize;
    while let Some(pos) = rest.find(&needle) {
        let abs = offset + pos;
        let before_ok = abs == 0
            || !haystack[..abs]
                .chars()
                .next_back()
                .expect("non-empty prefix: abs > 0 is a char boundary inside the haystack")
                .is_alphanumeric();
        let after = &rest[pos + needle.len()..];
        let after_ok = after.is_empty()
            || !after
                .chars()
                .next()
                .expect("non-empty remainder: emptiness was just checked")
                .is_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        offset = abs + needle.len();
        rest = after;
    }
    false
}

/// Decide whether an incoming message warrants a full processing cycle.
///
/// `tokens` are the mention names for this account (see [`mention_tokens`])
/// and `sent` is the tracker of message IDs the agent sent, used to
/// recognize replies. Under [`RelevanceFilter::Always`] every message is
/// relevant and neither is consulted.
pub fn is_message_relevant(
    config: &Config,
    account_name: &str,
    msg: &Message,
    tokens: &[String],
    sent: &SentMessageTracker,
) -> bool {
    match config.relevance_filter {
        RelevanceFilter::Always => true,
        RelevanceFilter::Smart | RelevanceFilter::Mentions => {
            // Direct messages always count — the user addressed the agent
            // personally. This mirrors resolve_routing's DM test so the two
            // never disagree about what a DM is.
            if msg.is_direct || msg.channel.is_none() {
                return true;
            }
            if tokens.iter().any(|t| contains_mention(&msg.content, t)) {
                return true;
            }
            if let Some(reply_to) = &msg.reply_to {
                let key = channel_key(account_name, msg.channel.as_deref());
                if sent.contains(&key, reply_to) {
                    return true;
                }
            }
            // Rule-tier miss. Under `Mentions` the caller drops the message;
            // under `Smart` the caller defers to the classifier (it must
            // distinguish the two, so both report the miss as `false`).
            false
        }
    }
}

/// System prompt for the relevance classifier: one word out, nothing else.
const CLASSIFIER_SYSTEM: &str = "You decide whether a group-chat message is \
directed at the named agent or asks for the agent's help. Reply with exactly \
one word: YES or NO.";

/// Prompt the classifier sees for one incoming message. Pure so it is unit
/// testable without a model call. `tokens` are the names/handles a human
/// might use to address the agent; they let the classifier recognize an
/// address the rule tier's substring test might have missed.
fn build_classifier_prompt(
    agent_name: &str,
    tokens: &[String],
    content: &str,
    channel: Option<&str>,
) -> String {
    let channel_hint = match channel {
        Some(c) => format!(" in the channel \"{c}\""),
        None => String::new(),
    };
    let alt: Vec<&str> = tokens
        .iter()
        .map(String::as_str)
        .filter(|t| *t != agent_name)
        .collect();
    let alt_hint = if alt.is_empty() {
        String::new()
    } else {
        format!(" People may also address you as: {}.", alt.join(", "))
    };
    format!(
        "You are {agent_name}, an AI agent in a group chat{channel_hint}.{alt_hint}\n\
A participant wrote:\n\n{content}\n\n\
Is this message addressed to you or asking for your help? Reply YES or NO."
    )
}

/// Turn the classifier's reply into a relevance verdict.
///
/// Fail-open: only an explicit "no" drops the message. Anything else — a
/// clear "yes", an empty reply, a refusal, a garbled token — processes it,
/// because dropping a message that was actually for the agent is worse than
/// spending a turn on noise.
fn parse_classifier_verdict(response: &str) -> bool {
    let first = response
        .trim()
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("");
    !first.eq_ignore_ascii_case("no")
}

/// LLM tier of the relevance filter (`RelevanceFilter::Smart`).
///
/// Called only when the rule tier has already failed to prove the message
/// relevant, and always from inside a processing task (never the poll loop,
/// which must stay responsive). Runs a one-shot completion and asks whether
/// the message is directed at the agent; any error in the model call fails
/// open so a provider hiccup cannot silently drop messages.
pub async fn classify_relevance(
    http: &reqwest::Client,
    model_ctx: &rustyclaw_core::gateway::ModelContext,
    config: &Config,
    msg: &Message,
    tokens: &[String],
    agent_name: &str,
) -> bool {
    let prompt = build_classifier_prompt(agent_name, tokens, &msg.content, msg.channel.as_deref());
    match model_ctx
        .complete(
            http,
            config.relevance_model.as_deref(),
            CLASSIFIER_SYSTEM,
            &prompt,
        )
        .await
    {
        Ok(reply) => parse_classifier_verdict(&reply),
        Err(e) => {
            warn!(
                error = %e,
                sender = %msg.sender,
                "Relevance classifier failed; processing message anyway"
            );
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyclaw_core::messengers::Message;

    fn msg(channel: Option<&str>, content: &str) -> Message {
        Message {
            id: "m1".into(),
            sender: "someone".into(),
            content: content.into(),
            timestamp: 0,
            channel: channel.map(str::to_string),
            reply_to: None,
            thread_id: None,
            media: None,
            is_direct: false,
            message_type: Default::default(),
            edited_timestamp: None,
            reactions: None,
        }
    }

    #[test]
    fn at_mention_is_recognized_case_insensitively() {
        assert!(contains_mention("hey @Ada can you look at this", "Ada"));
        assert!(contains_mention("hey @ada can you look at this", "Ada"));
        assert!(contains_mention("hi @ADA!", "Ada"));
        // "Ada" as a whole word counts even without the @; a bare substring
        // inside another word does not.
        assert!(contains_mention("hey Ada lover", "Ada"));
        assert!(!contains_mention("hey adaline", "Ada"));
    }

    /// Regression for Devin review #441 (BUG_0001, latest round): the @-form
    /// used to be a bare substring test, so "@adaline" or "bob@adaline.example"
    /// counted as addressing "ada". The character after the @-name must be a
    /// non-alphanumeric boundary, matching the whole-word path.
    #[test]
    fn at_mention_requires_a_boundary_after_the_name() {
        assert!(contains_mention("ping @ada", "Ada"));
        assert!(contains_mention("@ada, what do you think?", "Ada"));
        assert!(!contains_mention("ping @adaline", "Ada"));
        assert!(!contains_mention("mail bob@adaline.example", "Ada"));
        assert!(contains_mention("@ada!", "Ada"));
        // "_" is not alphanumeric, so it is a boundary here just as it is in
        // the whole-word path ("ada_x" matches "ada" there too).
        assert!(contains_mention("@ada_x", "Ada"));
    }

    #[test]
    fn whole_word_mention_is_recognized_but_not_words_containing_the_name() {
        assert!(contains_mention("Ada, what do you think?", "Ada"));
        assert!(contains_mention("I think Ada is right.", "Ada"));
        assert!(contains_mention("Ada", "Ada"));
        assert!(!contains_mention("adapter is broken", "Ada"));
        assert!(!contains_mention("madam", "Ada"));
    }

    /// Regression for Devin review #441 (BUG_0001): a name glued to itself
    /// inside a single word ("AdaAda") must not count as a mention — the
    /// second occurrence used to pass the boundary check because `pos == 0`
    /// was tested against the loop remainder, not the full haystack.
    #[test]
    fn name_repeated_inside_one_word_is_not_a_mention() {
        assert!(!contains_mention("AdaAda", "Ada"));
        assert!(!contains_mention("hey AdaAda!", "Ada"));
        assert!(!contains_mention("adaAda", "Ada"));
        // Same-name sequence split by a real boundary still counts.
        assert!(contains_mention("Ada, Ada", "Ada"));
        assert!(contains_mention("Ada Ada", "Ada"));
        // Longer needles are not fooled either.
        assert!(!contains_mention("rustyrusty", "rusty"));
        assert!(contains_mention("rusty, rusty", "rusty"));
    }

    #[test]
    fn empty_names_and_content_never_mention() {
        assert!(!contains_mention("hello", ""));
        assert!(!contains_mention("", "Ada"));
        assert!(!contains_mention("  ", "  "));
    }

    #[test]
    fn tracker_records_and_finds_sent_ids() {
        let mut tracker = SentMessageTracker::new();
        tracker.record("acct:room", "out-1");
        tracker.record("acct:room", "out-2");
        assert!(tracker.contains("acct:room", "out-1"));
        assert!(tracker.contains("acct:room", "out-2"));
        assert!(!tracker.contains("acct:room", "out-3"));
        assert!(!tracker.contains("other:room", "out-1"));
        assert!(!tracker.contains("acct:dm", "out-1"));
    }

    #[test]
    fn tracker_is_bounded_per_channel() {
        let mut tracker = SentMessageTracker::with_cap_for_test(3);
        for i in 0..5 {
            tracker.record("acct:room", &format!("out-{i}"));
        }
        assert!(!tracker.contains("acct:room", "out-0"));
        assert!(!tracker.contains("acct:room", "out-1"));
        assert!(tracker.contains("acct:room", "out-4"));
    }

    /// Regression for Devin review #441 (BUG_0001/job 2): the derived
    /// `Default` used to leave `cap` at 0, silently collapsing the tracker
    /// to a single entry per channel. `Default` must behave like `new()`.
    #[test]
    fn default_tracker_behaves_like_new() {
        let mut tracker = SentMessageTracker::default();
        assert_eq!(tracker.cap, TRACKER_CAP_PER_CHANNEL);
        for i in 0..(TRACKER_CAP_PER_CHANNEL + 10) {
            tracker.record("acct:room", &format!("out-{i}"));
        }
        assert!(tracker.contains("acct:room", "out-511"));
        assert!(tracker.contains("acct:room", "out-521"));
        assert!(!tracker.contains("acct:room", "out-0"));
    }

    #[test]
    fn dms_are_always_relevant() {
        let config = Config {
            relevance_filter: RelevanceFilter::Mentions,
            ..Config::default()
        };
        let tracker = SentMessageTracker::new();
        let mut direct = msg(None, "completely off-topic");
        direct.is_direct = true;
        assert!(is_message_relevant(&config, "acct", &direct, &[], &tracker));
        // A channel-less message is a DM even when the flag is not set.
        assert!(is_message_relevant(
            &config,
            "acct",
            &msg(None, "hi"),
            &[],
            &tracker
        ));
    }

    #[test]
    fn mentions_are_relevant_in_group_chats() {
        let config = Config {
            relevance_filter: RelevanceFilter::Mentions,
            ..Config::default()
        };
        let tracker = SentMessageTracker::new();
        let tokens = vec!["Ada".to_string(), "RustyClaw".to_string()];
        assert!(is_message_relevant(
            &config,
            "acct",
            &msg(Some("#room"), "please fix the build @Ada"),
            &tokens,
            &tracker
        ));
        assert!(is_message_relevant(
            &config,
            "acct",
            &msg(Some("#room"), "RustyClaw, what's the plan?"),
            &tokens,
            &tracker
        ));
        assert!(!is_message_relevant(
            &config,
            "acct",
            &msg(Some("#room"), "has anyone seen the docs?"),
            &tokens,
            &tracker
        ));
    }

    #[test]
    fn replies_to_agent_sent_messages_are_relevant() {
        let config = Config {
            relevance_filter: RelevanceFilter::Mentions,
            ..Config::default()
        };
        let mut tracker = SentMessageTracker::new();
        tracker.record(&channel_key("acct", Some("#room")), "out-9");

        let mut reply = msg(Some("#room"), "thanks, that worked");
        reply.reply_to = Some("out-9".into());
        assert!(is_message_relevant(&config, "acct", &reply, &[], &tracker));

        // A reply to someone else's message in the same channel is not.
        let mut other_reply = msg(Some("#room"), "agreed");
        other_reply.reply_to = Some("someone-elses-id".into());
        assert!(!is_message_relevant(
            &config,
            "acct",
            &other_reply,
            &[],
            &tracker
        ));

        // A reply to our message in a different channel is not relevant here.
        let mut cross_channel = msg(Some("#other"), "moving this");
        cross_channel.reply_to = Some("out-9".into());
        assert!(!is_message_relevant(
            &config,
            "acct",
            &cross_channel,
            &[],
            &tracker
        ));
    }

    #[test]
    fn always_mode_processes_everything() {
        let config = Config {
            relevance_filter: RelevanceFilter::Always,
            ..Config::default()
        };
        let tracker = SentMessageTracker::new();
        assert!(is_message_relevant(
            &config,
            "acct",
            &msg(Some("#room"), "unrelated chatter"),
            &[],
            &tracker
        ));
    }

    #[test]
    fn mention_tokens_include_platform_handles() {
        let config = Config::default();

        // IRC: explicit nick is a mention token; a display name with spaces
        // also contributes the sanitized form apply_profile would use.
        let irc = MessengerConfig {
            name: "irc-bot".into(),
            messenger_type: "irc".into(),
            nick: Some("clawbot".into()),
            ..Default::default()
        };
        let tokens = mention_tokens(&config, &irc);
        assert!(
            tokens.iter().any(|t| t == "clawbot"),
            "explicit IRC nick must be a token: {tokens:?}"
        );

        // IRC without an explicit nick: the sanitized display name counts.
        let irc_default = MessengerConfig {
            name: "irc-bot".into(),
            messenger_type: "irc".into(),
            ..Default::default()
        };
        let tokens = mention_tokens(&config, &irc_default);
        assert!(
            tokens.iter().any(|t| t == "RustyClaw"),
            "agent name must be a token: {tokens:?}"
        );

        // Matrix: the user-id local part is a mention token, so "@claw:homeserver"
        // group messages match under the mentions filter.
        let matrix = MessengerConfig {
            name: "matrix-bot".into(),
            messenger_type: "matrix".into(),
            user_id: Some("@claw:matrix.lunar.center".into()),
            ..Default::default()
        };
        let tokens = mention_tokens(&config, &matrix);
        assert!(
            tokens.iter().any(|t| t == "claw"),
            "Matrix local part must be a token: {tokens:?}"
        );
        assert!(
            tokens.iter().any(|t| t == "RustyClaw"),
            "agent name must be a token: {tokens:?}"
        );
        assert!(
            tokens.iter().any(|t| t == "Rusty Claw" || t == "RustyClaw"),
            "display name must be a token: {tokens:?}"
        );
    }

    // ── Classifier tier (#165) ────────────────────────────────────────────────

    #[test]
    fn classifier_prompt_names_the_agent_and_quotes_the_message() {
        let tokens = vec!["Ada".to_string(), "adaline".to_string()];
        let p = build_classifier_prompt("Ada", &tokens, "is anyone here?", Some("general"));
        assert!(p.contains("You are Ada"), "prompt: {p}");
        assert!(p.contains("\"general\""), "channel hint missing: {p}");
        assert!(p.contains("is anyone here?"), "content missing: {p}");
        assert!(p.contains("YES or NO"), "verdict instruction missing: {p}");
        assert!(p.contains("adaline"), "alternate names must be listed: {p}");

        let p = build_classifier_prompt("Ada", &[], "hi", None);
        assert!(!p.contains("channel"), "no channel hint expected: {p}");
        assert!(!p.contains("address you as"), "no alt names expected: {p}");
    }

    /// Only an explicit "no" drops; every other reply processes the message.
    #[test]
    fn classifier_verdict_is_fail_open() {
        assert!(parse_classifier_verdict("YES"));
        assert!(parse_classifier_verdict("yes"));
        assert!(parse_classifier_verdict("Yes, it is."));
        assert!(parse_classifier_verdict(""));
        assert!(parse_classifier_verdict("I cannot answer that"));
        assert!(parse_classifier_verdict("maybe"));

        assert!(!parse_classifier_verdict("NO"));
        assert!(!parse_classifier_verdict("no."));
        assert!(!parse_classifier_verdict("No, it isn't."));
        assert!(!parse_classifier_verdict("NO\n\nBecause reasons"));
    }

    /// Under Smart the rule tier reports a miss exactly like Mentions does
    /// (so the caller can distinguish "drop" from "classify"), and a DM or
    /// an @-mention still passes without a model call.
    #[test]
    fn smart_rule_tier_reports_misses_and_passes_direct_mentions() {
        let config = Config {
            relevance_filter: RelevanceFilter::Smart,
            ..Config::default()
        };
        let sent = SentMessageTracker::default();
        let tokens = vec!["Ada".to_string()];

        // DM: always relevant.
        let dm = msg(None, "hello there");
        assert!(is_message_relevant(&config, "acc", &dm, &tokens, &sent));

        // Group message with an @-mention: relevant, no model call needed.
        let mentioned = msg(Some("general"), "hey @Ada check this");
        assert!(is_message_relevant(
            &config, "acc", &mentioned, &tokens, &sent
        ));

        // Group message with no mention: a miss (caller decides classify).
        let noise = msg(Some("general"), "did anyone see the game last night");
        assert!(!is_message_relevant(&config, "acc", &noise, &tokens, &sent));
    }
}
