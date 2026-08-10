//! Message relevance pre-filter — the *rule* tier of RustyClaw#165.
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
//! The LLM classifier tier ("smart") is a separate follow-up that needs a
//! one-shot completion helper on `ModelContext`; it will live next to this
//! module and reuse the same decision point.

use std::collections::{HashMap, VecDeque};

use rustyclaw_core::config::{Config, MessengerConfig, RelevanceFilter};
use rustyclaw_core::messengers::Message;

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
#[derive(Debug, Default)]
pub struct SentMessageTracker {
    by_channel: HashMap<String, VecDeque<String>>,
    cap: usize,
}

impl SentMessageTracker {
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

/// Stable key for the sent-message tracker: account + channel, with DMs
/// folded into `dm` (DMs are always relevant anyway, so they are only ever
/// recorded defensively).
pub fn channel_key(account_name: &str, channel: Option<&str>) -> String {
    format!("{account_name}:{}", channel.unwrap_or("dm"))
}

/// The names a human would use to @-mention this agent on a given account:
/// the account's presented display name plus the configured agent name.
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

    // "@name" — the most common way to address a bot.
    if haystack.contains(&format!("@{needle}")) {
        return true;
    }

    // Whole-word match: "name:" after a call-out, "name," mid-sentence, or
    // the bare name on its own. The character before and after the match
    // must not be alphanumeric.
    let mut rest = haystack.as_str();
    while let Some(pos) = rest.find(&needle) {
        let before_ok = pos == 0 || !rest[..pos].chars().next_back().unwrap().is_alphanumeric();
        let after = &rest[pos + needle.len()..];
        let after_ok = after.is_empty() || !after.chars().next().unwrap().is_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
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
        RelevanceFilter::Mentions => {
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
            false
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

    #[test]
    fn whole_word_mention_is_recognized_but_not_words_containing_the_name() {
        assert!(contains_mention("Ada, what do you think?", "Ada"));
        assert!(contains_mention("I think Ada is right.", "Ada"));
        assert!(contains_mention("Ada", "Ada"));
        assert!(!contains_mention("adapter is broken", "Ada"));
        assert!(!contains_mention("madam", "Ada"));
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
        assert!(is_message_relevant(&config, "acct", &msg(None, "hi"), &[], &tracker));
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
        assert!(!is_message_relevant(&config, "acct", &other_reply, &[], &tracker));

        // A reply to our message in a different channel is not relevant here.
        let mut cross_channel = msg(Some("#other"), "moving this");
        cross_channel.reply_to = Some("out-9".into());
        assert!(!is_message_relevant(&config, "acct", &cross_channel, &[], &tracker));
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
}
