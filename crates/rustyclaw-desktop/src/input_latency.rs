//! Why this client touches keystrokes, and the guards that keep typing usable.
//!
//! Typing in the desktop window has gone jerky several times — characters
//! arriving in bursts, or being swallowed outright — always for the same
//! reason and never twice in the same place. The narrative version is in
//! `docs/input-latency.md`; this module is the enforcement.
//!
//! # The two mechanisms
//!
//! **`onkeydown` — shortcuts.** A bare `input`/`textarea` in a webview has no
//! submit behaviour, so Enter has to be wired up by hand for every field that
//! should accept it. These handlers compare one key and return; they cost
//! nothing and are not why typing ever felt slow. [`KEYBOARD_HOOKS`] lists
//! every one of them with its reason, and [`keyboard_hooks_are_documented`]
//! fails if an undocumented one appears. Nothing listens on `document` or
//! `window` — a global key hook would see every keystroke in the window, and
//! there has never been a reason to want that.
//!
//! **`oninput` — controlled inputs.** This is the expensive one, and it is not
//! optional: `oninput` mirrors what was typed into a Rust signal, and the
//! field's `value` is rendered back from that signal. Rust needs the text (to
//! send the message, to connect, to save a rename) and needs to be able to set
//! it (clearing the composer after send). The consequence is that **one
//! keystroke triggers a render whose output is written back onto the field**.
//! A slow render is felt as lag; a render that began before the last keystroke
//! carries a stale `value` and eats the character typed in the meantime.
//!
//! So the rule is not "stop hooking input". It is:
//!
//! 1. Draft text lives in the smallest component that renders the field —
//!    never in `App`, whose scope re-renders the transcript and the sidebar.
//! 2. Per-keystroke work must not scale with the length of the conversation.
//! 3. Streaming must not re-render at token rate.
//!
//! The tests below assert all three.

/// One file's worth of keyboard shortcut handlers, and why they are there.
struct HookSite {
    /// Path relative to the crate's `src/`.
    file: &'static str,
    /// How many `onkeydown:`/`onkeyup:`/`onkeypress:` attributes it has.
    count: usize,
    /// What the handlers do, and why the field needs them.
    reason: &'static str,
}

/// Every keyboard shortcut handler in the crate.
///
/// All of them are "Enter submits this field", except the inline question card
/// which also takes Escape and the arrow keys. Enter-to-send in the composer
/// is not here: that `textarea` belongs to `dioxus-genai-chat`, which handles
/// Enter (and Shift+Enter for a newline) itself.
///
/// Adding a handler means adding it here. Prefer not adding one at all: if a
/// field lives in a `form`, the browser's own submit behaviour is free.
const KEYBOARD_HOOKS: &[HookSite] = &[
    HookSite {
        file: "app/dialogs.rs",
        count: 1,
        reason: "TOTP code field: Enter authenticates instead of doing nothing.",
    },
    HookSite {
        file: "components/connection.rs",
        count: 1,
        reason: "Gateway URL field: Enter connects.",
    },
    HookSite {
        file: "components/credential_request.rs",
        count: 1,
        reason: "Credential field: Enter submits the requested secret.",
    },
    HookSite {
        file: "components/edit_dialogs.rs",
        count: 2,
        reason: "Edit-project path and edit-thread caption fields: Enter saves.",
    },
    HookSite {
        file: "components/hatching.rs",
        count: 1,
        reason: "Agent-name field: Enter advances the hatching flow.",
    },
    HookSite {
        file: "components/new_project.rs",
        count: 1,
        reason: "New-project name field: Enter creates the project.",
    },
    HookSite {
        file: "components/secrets.rs",
        count: 2,
        reason: "Reveal-code and add-secret fields: Enter submits.",
    },
    HookSite {
        file: "components/settings.rs",
        count: 1,
        reason: "Provider API-key field: Enter saves the credential.",
    },
    HookSite {
        file: "components/sidebar.rs",
        count: 2,
        reason: "Inline project and thread rename boxes: Enter commits the name.",
    },
    HookSite {
        file: "components/user_prompt.rs",
        count: 3,
        reason: "Inline question card: one handler on the card (Enter answers, \
                 Escape dismisses, arrows move a single-select highlight — keydown \
                 bubbles, so one handler covers every prompt type), plus two on the \
                 button rows that stop Enter bubbling, so Enter on a focused button \
                 activates that button rather than submitting the default answer.",
    },
    HookSite {
        file: "components/vault_unlock.rs",
        count: 1,
        reason: "Vault passphrase field: Enter unlocks.",
    },
];

/// Attribute forms that hook a key event.
const KEY_ATTRS: &[&str] = &["onkeydown:", "onkeyup:", "onkeypress:"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_transcript::{markdown_parse_count, to_transcript};
    use rustyclaw_core::ui::ChatMessage;
    use rustyclaw_view::ChatSurfaceData;
    use std::path::{Path, PathBuf};

    /// Sanitiser-run counting is process-global, so the two tests that measure
    /// it must not interleave.
    static PARSE_COUNT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn src_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    /// Every `.rs` file under `src/`, as (path relative to `src/`, contents).
    /// This file is skipped: it names the attributes it is looking for.
    fn sources() -> Vec<(String, String)> {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
            let entries = std::fs::read_dir(dir).expect("src/ is readable");
            for entry in entries {
                let path = entry.expect("readable dir entry").path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let rel = path
                        .strip_prefix(root)
                        .expect("path is under src/")
                        .to_string_lossy()
                        .replace('\\', "/");
                    if rel == "input_latency.rs" {
                        continue;
                    }
                    let body = std::fs::read_to_string(&path).expect("source is readable");
                    out.push((rel, body));
                }
            }
        }

        let root = src_dir();
        let mut out = Vec::new();
        walk(&root, &root, &mut out);
        out.sort();
        out
    }

    fn count_occurrences(haystack: &str, needle: &str) -> usize {
        haystack.matches(needle).count()
    }

    /// A keyboard shortcut that is not in [`KEYBOARD_HOOKS`] fails the build.
    ///
    /// The point is not that hooking keys is forbidden — it is that each one
    /// should be a deliberate choice someone can read the reason for, so the
    /// set stays as small as the UI actually needs.
    #[test]
    fn keyboard_hooks_are_documented() {
        let mut found: Vec<(String, usize)> = Vec::new();
        for (file, body) in sources() {
            let count: usize = KEY_ATTRS
                .iter()
                .map(|attr| count_occurrences(&body, attr))
                .sum();
            if count > 0 {
                found.push((file, count));
            }
        }

        for site in KEYBOARD_HOOKS {
            assert!(
                site.reason.len() > 20,
                "{} is listed without a real reason. The list is only worth \
                 having if it says why each handler exists.",
                site.file
            );
        }

        let mut documented: Vec<(String, usize)> = KEYBOARD_HOOKS
            .iter()
            .map(|site| (site.file.to_string(), site.count))
            .collect();
        documented.sort();

        assert_eq!(
            found, documented,
            "keyboard hooks in the source no longer match KEYBOARD_HOOKS.\n\
             Found: {found:?}\nDocumented: {documented:?}\n\
             Add the handler to KEYBOARD_HOOKS with the reason it exists (or \
             remove the handler). See docs/input-latency.md."
        );
    }

    /// No handler on `document`/`window`, in any injected script.
    ///
    /// A global key listener sees every keystroke in the window, including
    /// every character typed into every field, and runs before the field does.
    /// The client injects JavaScript for link interception and auto-scroll;
    /// neither needs keys, and nothing else should.
    #[test]
    fn no_global_key_listeners() {
        for (file, body) in sources() {
            for (idx, _) in body.match_indices("addEventListener(") {
                let tail: String = body[idx + "addEventListener(".len()..]
                    .chars()
                    .take(40)
                    .filter(|c| !c.is_whitespace())
                    .collect();
                assert!(
                    !(tail.starts_with("'key") || tail.starts_with("\"key")),
                    "{file} installs a global key listener ({tail}). Keystrokes \
                     belong to the field that receives them — hook the element, \
                     not the document. See docs/input-latency.md."
                );
            }
        }
    }

    /// Dialogs must render in their own reactive scope, and `App` must not own
    /// draft text.
    ///
    /// `render_dialogs(sig)` was a plain function call inlined into `App`'s
    /// scope, so the TOTP field and the agent-name field — whose signals were
    /// declared in `App` — re-rendered the entire window on every character,
    /// re-cloning the message list into `Chat`'s props each time.
    #[test]
    fn dialogs_render_in_their_own_scope() {
        let sources = sources();
        let get = |name: &str| {
            sources
                .iter()
                .find(|(file, _)| file == name)
                .map(|(_, body)| body.clone())
                .unwrap_or_else(|| panic!("{name} exists"))
        };

        let app = get("app/mod.rs");
        assert!(
            !app.contains("render_dialogs("),
            "App renders the dialogs inline again. Inlined, their draft-text \
             signals re-render App — and so the transcript and sidebar — on \
             every keystroke. Render them as the `Dialogs` component."
        );
        assert!(
            app.contains("Dialogs {"),
            "the `Dialogs` component is no longer mounted from App."
        );
        assert_eq!(
            KEY_ATTRS
                .iter()
                .chain(std::iter::once(&"oninput:"))
                .map(|attr| count_occurrences(&app, attr))
                .sum::<usize>(),
            0,
            "App renders a text field directly. A field's draft signal belongs \
             to the smallest component that renders it; in App's scope every \
             keystroke re-renders the whole window. See docs/input-latency.md."
        );

        let signals = get("app/signals.rs");
        for draft in ["auth_code", "hatching_dialog"] {
            assert!(
                !signals.contains(draft),
                "`{draft}` is back in AppSignals. Draft text held there is read \
                 in App's scope, which is what made typing re-render the window."
            );
        }
    }

    fn assistant(content: &str, streaming: bool) -> ChatMessage {
        let mut msg = ChatMessage::start_assistant(format!("id-{content}"));
        msg.content = content.to_string();
        msg.is_streaming = streaming;
        msg
    }

    /// A keystroke in the composer must not re-parse the conversation.
    ///
    /// The composer's draft signal lives in `Chat`, the same component that
    /// builds the transcript, so `to_transcript` runs on every character
    /// typed. Sanitising a message is an HTML parse plus a CommonMark parse;
    /// doing that for every message in a long thread, per keystroke, is what
    /// jerky typing has felt like every time it came back.
    #[test]
    fn rebuilding_an_unchanged_thread_parses_no_markdown() {
        let _guard = PARSE_COUNT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let surface = ChatSurfaceData::default();
        let thread: Vec<ChatMessage> = (0..40)
            .map(|i| {
                assistant(
                    &format!("unchanged-thread bubble {i} with **markdown**"),
                    false,
                )
            })
            .collect();

        // First build parses each bubble once.
        let before = markdown_parse_count();
        to_transcript(&thread, &surface, false);
        assert_eq!(
            markdown_parse_count() - before,
            thread.len(),
            "the first build should parse each assistant bubble exactly once"
        );

        // Every rebuild after that — i.e. every keystroke — parses nothing.
        let warm = markdown_parse_count();
        for _ in 0..5 {
            to_transcript(&thread, &surface, false);
        }
        assert_eq!(
            markdown_parse_count(),
            warm,
            "re-rendering an unchanged thread re-parsed its markdown. Typing in \
             the composer rebuilds the transcript on every keystroke, so this is \
             per-character cost that grows with the conversation. See \
             docs/input-latency.md."
        );
    }

    /// While a reply streams, only the bubble that changed may be re-parsed.
    ///
    /// The UI commits streaming chunks about twelve times a second. If each of
    /// those flushes re-parsed the whole thread, typing during a reply would
    /// stutter exactly the way it used to.
    #[test]
    fn streaming_reparses_only_the_streaming_bubble() {
        let _guard = PARSE_COUNT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let surface = ChatSurfaceData::default();
        let mut thread: Vec<ChatMessage> = (0..40)
            .map(|i| {
                assistant(
                    &format!("streaming-thread bubble {i} with **markdown**"),
                    false,
                )
            })
            .collect();
        thread.push(assistant("streaming-thread reply", true));

        to_transcript(&thread, &surface, false);

        // Three chunks land; each may re-parse the growing bubble and nothing
        // else. A streaming bubble is not cached — its text changes every
        // flush — so one parse per flush is the floor, not a regression.
        let warm = markdown_parse_count();
        for chunk in ["…one", "…two", "…three"] {
            let last = thread.last_mut().expect("streaming bubble");
            last.content.push_str(chunk);
            to_transcript(&thread, &surface, false);
        }
        assert_eq!(
            markdown_parse_count() - warm,
            3,
            "a streaming flush re-parsed settled bubbles as well as the live \
             one. Only the bubble whose text changed may be re-parsed. See \
             docs/input-latency.md."
        );
    }
}
