# Typing latency in the desktop client

Typing in the desktop client has gone jerky — characters arriving late, in
bursts, or being swallowed — several times, always for the same underlying
reason and never in the same place twice. This document says why the client
touches keystrokes at all, what the rules are, and which tests hold them.

If you are here because typing feels bad again, jump to
[Diagnosing a regression](#diagnosing-a-regression).

## Why the client hooks keys at all

Two distinct mechanisms, with very different costs.

### 1. `onkeydown` — keyboard shortcuts

Sixteen handlers across the crate, all the same shape: Enter submits the field
(and, in the inline question card, Escape dismisses and the arrow keys move the
selection). They exist because a bare `input`/`textarea` in a webview has no
submit behaviour of its own — without them, Enter in the gateway URL box, the
TOTP field, a rename box or the composer does nothing at all.

These are cheap and are **not** the cause of any latency problem: the handler
compares one key, and for anything that is not Enter/Escape/an arrow it returns
immediately. Every one of them is listed with its reason in
`crates/rustyclaw-desktop/src/input_latency.rs`, and a test fails if a new one
appears that is not in that list. No handler is registered on `document` or
`window`: a global key listener would see every keystroke in the window, and we
have never needed one.

### 2. `oninput` — controlled inputs

This is the mechanism that goes wrong, and it is not optional. Every text field
is a *controlled* input: `oninput` copies what was typed into a Rust signal, and
the field's `value` attribute is rendered back from that signal. Rust needs the
text (to send the message, connect to the gateway, save the rename) and the app
needs to be able to set it (clearing the composer after send, prefilling a
rename box).

The cost is that **one keystroke triggers a render**, and the rendered `value`
is then written back onto the field. Two failure modes follow:

- **Slow render → jerky typing.** The keystroke round-trips through Rust before
  the character settles.
- **Slow render → lost characters.** A render that started before the last
  keystroke carries a stale `value`; applying it to the field overwrites what
  was typed in the meantime.

So the rule is not "avoid hooking input" — it is **the render a keystroke
triggers must be small, and must not grow with the size of the conversation.**

## The rules

### Draft text lives in the smallest scope that renders the field

A `use_signal` written on every keystroke re-renders the scope that declares
it, and every scope that reads it. Declaring one in `App` means one character
re-renders the entire window.

This is exactly what happened to the dialogs: `auth_code` (the TOTP field) and
`hatching_dialog` (the agent-name field) were declared in `App` and rendered by
a plain `render_dialogs(sig)` call, which is inlined into `App`'s reactive
scope. Typing one digit of a TOTP code re-cloned the whole message list into
`Chat`'s props and rebuilt the sidebar tree.

Both signals now live in `Dialogs`, which is a real `#[component]` with its own
scope. `AppSignals` — the bundle `App` hands to `Dialogs` — carries no draft
text, and its doc comment says so. That is compiler-enforced: `App` cannot
subscribe to a signal it does not have.

### Per-keystroke work must not scale with the conversation

`chat_transcript::to_transcript` walks every message in the thread and, for
assistant messages, runs `ammonia::clean` (an HTML parse) followed by
`markdown_prep::prepare` (a CommonMark parse) over each one. It is called from
`Chat`, which is the component that owns the composer's draft signal — so it
runs on **every keystroke in the composer**, over the whole conversation.

Sanitising is pure, so results are cached by source text in `MarkdownCache`.
Typing into the composer now re-parses nothing; a streaming reply re-parses
only the bubble that changed (a streaming bubble is deliberately not cached,
since its text changes on every flush).

### Streaming must not re-render at token rate

Committing each streaming chunk straight to app state re-rendered the window
once per token, and every controlled input in it got a stale `value` patch at
that rate. Chunks are coalesced in the UI updater and committed on an ~80 ms
budget (`app::flush_pending_chunks`). Ordering is preserved by flushing before
any non-chunk event.

## Diagnosing a regression

Symptoms and where to look, in the order worth checking:

1. **Jerky only while a reply streams** → the streaming throttle. Check
   `flush_pending_chunks` and its callers in `app/mod.rs`.
2. **Jerky in the composer, worse in long threads** → something in `Chat`'s
   render got expensive again, or the markdown cache is missing. Run the
   latency guard tests.
3. **Jerky in a dialog or the sidebar rename box** → a draft signal escaped
   into a wider scope. Look for a `use_signal` in `App` (or in `AppSignals`)
   that an `oninput` writes to.
4. **Characters vanishing rather than lagging** → a stale `value` patch. Same
   causes as above; it is the same bug further along.

## The tests that hold this

In `crates/rustyclaw-desktop/src/input_latency.rs`:

- `keyboard_hooks_are_documented` — every `onkeydown`/`onkeyup`/`onkeypress` in
  the crate must appear in the registry with a reason, and the counts must
  match. Adding a handler without documenting it fails the build.
- `no_global_key_listeners` — no `addEventListener("key…")` in any injected
  JavaScript.
- `dialogs_render_in_their_own_scope` — `App` must not render the dialogs
  inline, and `AppSignals` must not carry draft text.
- `rebuilding_an_unchanged_thread_parses_no_markdown` and
  `streaming_reparses_only_the_streaming_bubble` — the per-keystroke and
  per-flush costs, asserted directly by counting sanitiser runs.

`rustyclaw-desktop` is in the CI unit-test job, so these actually run. It was
not, which is a large part of why this kept coming back unnoticed.
