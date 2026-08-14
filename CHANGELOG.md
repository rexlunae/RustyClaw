# Changelog

All notable changes to RustyClaw will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Plugin UI events and tool groups on the wire (plugin architecture phase 0,
  continued).** Clicking a plugin action button used to do exactly one thing:
  put a chat message in front of the model asking it to please run the action
  — the click itself vanished. A new `PluginUiEvent` client frame now carries
  the interaction to the gateway, which records it against the plugin (a
  bounded `_ui_events` ring inside its persisted state, replicated to clients
  with the rest of the state) and replies with the refreshed plugin list; the
  desktop dock sends it on every action press. Declarative actions are still
  prose for the agent, so the ask remains as the explicit second half —
  native plugins will consume the event in `on_event` and drop the prompt.
  The catalog's in-tree tool groups are also reachable over the wire:
  `ToolGroupsList` returns each group with its enabled state and tool count,
  `ToolGroupSetEnabled` flips one (applied to the live catalog, persisted in
  `disabled_tool_groups`, refreshed list in reply) — the surface the plugin
  manager UI will drive. New frames are appended with pinned discriminants
  (100–102 client, 95 server), keeping the positional bincode encoding
  compatible for existing peers.

- **Runtime tool catalog (plugin architecture phase 0, `docs/PLUGIN_ARCHITECTURE.md` §5).**
  The agent's toolset was a hardcoded list of static definitions, with three
  side tables (parameters, summaries, panel categories) keyed on tool names —
  nothing could add or remove a tool while the gateway ran, which the plugin
  system needs and which blocked the plan to ship built-ins as in-tree
  plugins. Tools now live in a `ToolCatalog`: built-ins register at startup
  in named in-tree groups (all enabled by default; the group names are the
  categories the tool-config panel already shows), each registration
  resolving its parameter schema, summary, and category once. Provider
  schemas, tool execution, the tool-config panel, and the toggle validator
  all read catalog snapshots, so a tool registered at runtime — the plugin
  loader lands in a later phase — is advertised to the model on the next
  turn, executable, and toggleable like any built-in, and an unregistered
  or group-disabled tool fails as unknown instead of executing while
  unadvertised. New config: `disabled_tool_groups = [...]` disables whole
  groups (not advertised, not executable); unknown names warn rather than
  fail the boot. Plugin tool names are namespaced by enforced
  `<plugin>_` prefix, and registration is atomic — one bad name rejects the
  batch.

- **Self-healing tool calls (#462).** Models damage their own tool calls in
  recurring, mechanical ways, and each used to be handled by failing. A new
  `tool_healing` layer at the provider choke points now repairs what is
  unambiguous: malformed argument JSON is unwrapped (markdown fences,
  surrounding prose), de-comma'd and completed (truncation cuts mid-string
  or mid-object) before the old `{}` fallback; argument values are coerced
  toward the tool's declared parameter type when lossless (number ↔ numeric
  string, bool ↔ `"true"`/`"false"`); exact duplicate calls within one
  response collapse to one; tool-call XML leaked into the text channel is
  stripped, including tags split across streaming chunks; and a call
  repeated verbatim 5+ rounds in a row gets a note telling the model it is
  looping (warn, never block — polling is legitimate). All behind
  `[tool_healing] enabled` (default on).

- **Curated per-model inference defaults (#464).** Model families that
  publish recommended sampling settings (Qwen, QwQ, DeepSeek-R1, GLM, Kimi,
  Llama 3, Gemma) now get them applied automatically when served through
  endpoints that don't do it themselves (Ollama, LM Studio, OpenRouter, any
  OpenAI-compatible proxy). Explicit `[model]` config wins field by field;
  `<settings_dir>/model_defaults.toml` overrides the built-in table without
  a rebuild; `rustyclaw model defaults [model]` shows what applies and why
  (each built-in entry names its source). Claude/GPT/Gemini are deliberately
  absent — their providers already default correctly. `top_p` is now a
  first-class `[model]` option alongside `temperature`.

- **User override for exfiltration-guard blocks (#418).** The
  credential-exfiltration guards were absolute, and the command-pattern
  guard is a heuristic with false positives (`cat ~/.rustyclaw/config.toml`
  while debugging blocked the same as a key grab). A guard block on an
  interactive connection is now put to the user — which tool, the exact
  arguments, the guard's reason, default deny — and approval re-runs that
  one call inside an override scope that dies with it. Headless callers
  (cron, triggers, messengers, subagents) have nobody to ask and keep the
  absolute guards; `guard_override_prompts = false` restores the old
  behaviour everywhere. Null-byte/length command checks are not
  overridable, nor are sandbox path denials or vault access policies.

- **Gateway controls in the desktop client (#414).** A new "Gateway…" entry
  under Tools (Ctrl/Cmd+G) opens a panel split in two. The top half manages
  the gateway daemon *on this machine* — start, stop and restart, backed by
  the same `rustyclaw_core::daemon` calls `rustyclaw gateway start|stop|restart`
  uses, with the daemon's status, SSH listen address and log path shown
  alongside. The bottom half describes the gateway this client is connected
  to and offers the one lifecycle command the wire protocol carries and that
  needs no input from anyone: reload configuration, which works against a
  remote gateway as well as a local one. The split is deliberate — the panel
  states which of the two each control acts on, so "Stop" cannot read as
  stopping a remote host. The panel never asks for the vault password, so a
  daemon started from it comes up with the vault locked; the panel says so,
  and the existing unlock dialog handles it over the session.

  Also fixed while wiring it up: the desktop resolved several config-derived
  values with `Config::load(None)`, which re-reads the default location and
  discards `--config` / `--settings-dir` / `--profile`. Those now use the
  config `main` actually resolved, so launching under `--profile` no longer
  shows the default profile's model or asks an already-hatched agent to
  hatch again.

- **`boot.toml` is now created automatically (#175, migration rungs 3–4).**
  The boot/extended config split shipped with nothing that ever *wrote*
  `boot.toml` — `BootConfig` had no `save`, onboarding did not create one, and
  `Config::save` wrote only `config.toml`. Every install was therefore still
  single-file, and the resilience the split exists for had never engaged for
  anybody: a torn `config.toml` cost the user every setting they had, the
  vault's `secrets_password_protected` flag among them, which is one way the
  gateway ends up starting without asking for a passphrase. `Config::load`
  now derives the boot slice from a config that loaded successfully and
  writes it the first time it finds `boot.toml` missing, so existing installs
  migrate themselves on next start. Deliberately *not* from a config that did
  not load: the missing-file and quarantined-file paths leave `config` as
  defaults, and writing those into the file that outranks `config.toml` would
  turn one bad boot into a permanent one. A boot slice with nothing in it is
  not written at all, `ssh_bind` is recorded only for a config that really has
  an `[ssh]` section (rather than inventing the built-in default), and a write
  that fails is a warning, never a failed start — a safety net that can stop
  the gateway from starting is worse than no safety net.

- **Message relevance filter — rule tier (`relevance_filter = "mentions"`).**
  In group chats, every message previously triggered a full agent response
  cycle, burning tokens on chatter that was never directed at the agent.
  The new opt-in `relevance_filter = "mentions"` config value (default
  `"always"`, preserving historic behavior) drops group-chat messages that
  neither mention the agent by name (`@Name` or the name as a whole word,
  case-insensitive) nor reply to a message the agent sent. Direct messages
  always pass. Skipped messages are logged at debug level and never touch
  the model, history, or typing indicators. The sent-message IDs the
  gateway produces are tracked per channel (bounded) to recognize replies.
  The LLM classifier tier (`"smart"`) from #165 remains a follow-up: it
  needs a one-shot completion helper on `ModelContext` and will extend the
  same decision point.

- **Boot config (`boot.toml`)** — first two rungs of the #175 migration
  path. Boot-critical fields (model provider, workspace path, SSH transport
  bind) can live in a small, stable `<settings_dir>/boot.toml`:
  `[provider] name/model`, `[workspace] path` (with `~` expansion),
  `[gateway] ssh_bind`. When present, boot.toml wins for those fields
  (preserving a custom provider's `base_url`); when absent, the legacy
  single-file behavior is unchanged. Deterministic recovery before giving
  up: TOML parse failure falls back to JSON (wrong-extension file), an
  invalid bind is dropped with a warning, and only a truly unreadable file
  is fatal — it is moved aside with a clear message so the next boot does
  not loop. The resilience win: a corrupt extended `config.toml` degrades
  to defaults *anchored at the corrupt file's directory*, then the boot
  slice still lands, so the gateway can still reach an LLM and self-heal.
  API keys are not in boot.toml; they keep resolving per-provider from the
  vault or `*_API_KEY` env vars.

- **Messenger setup in the clients: credentials, profile, and thread routing.**
  Messengers were configurable only by hand-editing `[[messengers]]` in
  `config.toml`, which meant live bot tokens sitting in plaintext next to
  everything else. Both clients now have a setup panel — `/messengers` in
  the TUI, *Tools → Messenger Setup…* on the desktop — covering three
  things. **Credentials** go to the encrypted vault under
  `messenger/<account>/<field>`; config keeps only a reference
  (`secret_refs`), and the gateway resolves it at connect time. Values
  travel one way: a credential is sent when you type it and is never
  returned, so a client cannot display or leak one. **Profile** is the
  name and description the agent presents on each messenger, defaulting to
  the agent's own name and description and overridable per account; it
  reaches the platform where the backend allows it (`set_text_status`,
  `set_profile_picture`, IRC nick) and reaches the model always, via a new
  identity section in the messenger system prompt. **Thread routing** is a
  new `[[messenger_routes]]` table binding `(messenger, channel)` to a
  gateway thread: a routed channel adopts that thread's conversation key
  and working directory, so two channels pointed at one thread share a
  conversation and tools run where the thread lives. Channel-specific
  routes outrank account-wide ones; unrouted channels behave exactly as
  before. What each backend needs is described once, as data, in
  `rustyclaw_core::messengers::setup::KINDS` — both clients render their
  forms from it, so a new messenger type is one entry rather than a form
  per client.

- **Existing plaintext credentials are flagged, not seized.** Accounts
  carrying secrets in `config.toml` keep working and are marked in the
  panel with a per-account "move to vault" action. Migration writes to the
  vault before clearing the plaintext copy, so an interrupted move loses
  nothing.

- **Focused subagents: narrow profiles, restricted toolsets, real runs.**
  The main agent can now delegate well-scoped work to *focused subagents*
  via a new `subagent_run` tool. A subagent runs from a **profile** — a
  tight, job-specific system prompt plus an explicit tool allowlist — and
  starts with no conversation history: it sees only the task and the
  context the main agent explicitly feeds it, and its model calls only
  present the profile's tools (a new `ProviderRequest::allowed_tools`
  filter; previously every model call carried the entire ~120-tool
  registry). Built-in profiles cover common jobs — `code-writer`,
  `code-reviewer`, `bug-hunter`, `test-writer`, `researcher`,
  `doc-writer` — and the main agent can define custom profiles on demand
  with `subagent_create` (persisted under `<settings_dir>/subagents/`,
  validated against the tool registry; interactive, agent-management, and
  installation-level tools are rejected from subagent toolsets).
  `subagent_list` shows every profile with its toolset, `subagent_delete`
  removes custom ones. Runs execute in a headless gateway tool loop that
  enforces the allowlist and the user's per-tool permission policy
  (anything not `Allow` is refused — no user is present to approve),
  honors the shared rate limiter, records the run in the session manager
  (visible via `sessions_list` / `sessions_history`), and returns the
  subagent's final report to the parent as the tool result. Thread and
  compaction summarisation requests now present no tools at all via the
  same mechanism.

- **Engines dialog: per-engine tabs and live install output.** The Local
  Engines & Models dialog now renders one tab per detected engine (←/→ or
  Tab to switch in the TUI; a Bulma tab strip on desktop), so each
  engine's status, models, and actions have their own focused view instead
  of one long combined list. Engine installs — which previously ran
  silently — now stream their output live: `execute`-style installers
  (`curl … | sh`, `brew install`, `cargo install …`) are read line by
  line via a new `stream_shell` helper in core, forwarded over a new
  `EngineActionProgress` frame (mirroring how model-pull progress already
  streams), and folded into the installing engine's tab as a bounded,
  live-updating log that ends with "install complete" / "install failed".
  The install output is tracked per engine and survives the frequent
  engine-list refreshes.

- **Live display and inline controls for long-running processes.** While
  a tool call executes, the gateway now streams a `ToolStatus` frame
  every second (after a 2s grace period so fast tools stay silent)
  carrying the call's elapsed time and — when the tool is waiting on a
  child process — that process's CPU usage, resident memory, and
  scheduler state (running, sleeping, blocked on I/O, paused, …),
  sampled via a new exec-status registry that every foreground
  `execute_command` child registers with. The TUI renders this as a
  live line inside the inline tool panel (`⏳ 12s · running · cpu 87% ·
  mem 145 MB · pid 4242`), the desktop shows the same line under the
  running tool call, and the CLI prints periodic status to stderr.
  When the status carries a PID the process is controllable inline from
  the chat — Ctrl+Z pauses/resumes (SIGSTOP/SIGCONT, with the exec
  timeout clock frozen while paused so a paused command can't time
  out), Ctrl+T sends SIGTERM, Ctrl+K sends SIGKILL — via a new
  `ProcessControl` client frame that the gateway's reader task handles
  even while the tool loop is blocked on that very process. Controls
  are allowlisted: only PIDs the gateway itself spawned for the current
  tool call can be signalled, and exec children now lead their own
  process group so signals reach the whole shell pipeline.
- **Live tool activity.** Running commands now show their output as it
  happens, inside the same panel as the tool call. `execute_command`
  reads the child's pipes incrementally and the gateway forwards each
  chunk over the previously-stubbed `ToolOutputDelta` frame; both
  clients fold the chunks into the running call's panel, which stays
  open while running (running work is what you want to watch) and
  collapses to the compact one-liner when it finishes. Output is
  rendered the way a terminal would: `\r`-redrawing progress bars
  overwrite their line in place instead of stacking hundreds of lines,
  ANSI color/cursor escapes are stripped, and the live tail is bounded
  (last 40 lines). The desktop also stops emitting a separate "tool
  result" bubble per call — the invocation, live progress, duration,
  and final result are one component now, halving transcript noise for
  agentic work.

- **The agent explains itself: visible reasoning, compact tool activity,
  timings.** The gateway has always streamed the model's reasoning text
  over the wire, but the client event layer discarded it — users only
  ever saw a spinner. `GatewayEvent::ThinkingDelta` now carries the
  text, and both clients accumulate it into a collapsible 💭 block:
  compact by default (a one-line gist under a "Thought for 4.2s"
  header) and fully expandable (Ctrl+E in the TUI; the desktop renders
  a step-per-paragraph reasoning timeline). Reasoning also folds the
  moment answer text starts streaming instead of at stream end.
  Tool calls in the TUI drop the raw-JSON peek for a semantic one-liner
  (`read src/main.rs:10–80 · ✓ 0.4s · 71 lines`, `$ cargo test · ✓
  12s`) with argument/result detail on expand, matching the desktop's
  hint panels; both clients stamp every tool call and thinking block
  with its client-measured wall-clock duration.

- **Usage analytics and logs panels backed by real telemetry.** The
  gateway now installs a stats-collecting observer at startup (it
  previously passed `None`, so the observability layer recorded
  nothing): every LLM call is recorded with provider, model, token
  counts (the genai backend's captured usage now actually reaches the
  telemetry — it was a `TODO`), latency, and outcome, alongside a
  human-readable ring of LLM/tool/channel/error events. `/analytics
  [day|week|month|all]` and `/logs [source] [n]` in the TUI and the
  View-menu Usage Analytics / Logs dialogs on desktop query it; the
  logs panel also serves managed-service logs by service name.
- **Desktop custom-provider management.** Settings gains a Custom
  Providers section — list/remove existing `[[custom_providers]]`
  entries and add new ones (id, name, base URL, API format, key secret,
  static models) with validation; saving updates the provider catalogue
  so the model bar picks the change up immediately. Also: the Skills
  menu opens a real skills manager (was "coming soon"), the secrets
  dialog's Add Secret flow works (with auto-refresh after every vault
  mutation and a real 2FA indicator via the new `SecretsHasTotp`
  client command), the Services dialog populates on open, and System
  Info fetches host/load data on open.
- **The cron, memory, MCP, channels, and tool-config panels are real.**
  The gateway's panel handler previously returned stub/empty responses
  for every panel request even though the backing subsystems existed.
  Panels now operate on the same backends the AI tools use: cron
  list/add/pause/resume/remove against the persistent `.cron` store,
  memory list/add/edit/delete against `MEMORY.md` (bullets as entries,
  `##` headings as categories) plus `HISTORY.md` search, MCP server
  list/connect/disconnect via a shared `McpManager` (the documented
  `[mcp.servers.*]` config section is now actually loaded, and ad-hoc
  connects persist to it), tool enable/disable via
  `config.tool_permissions`, and messenger channel status/pair/unpair
  via the `[[messengers]]` config.
- **TUI:** `/cron`, `/memory`, `/mcp`, and `/channels` now open live
  panels (they used to tab-complete and then report "Unknown command"),
  with subcommands for mutations (`/cron add <name> | <schedule> |
  <message>`, `/memory add [category ::] <content>`, `/mcp connect
  <name> [command…]`, `/channels pair|unpair <name>`, …) and
  auto-refresh after every change.
- **Desktop:** the Tools menu gains Scheduled Jobs, Memory, MCP
  Servers, Channels, and Tool Permissions dialogs — the last is the
  desktop's first tool-management surface.

- **User-configured custom model providers.** New `[[custom_providers]]`
  config section (id, display name, base URL, API format, optional API-key
  secret, optional static model list). Entries are registered into the
  provider catalogue at load time (`providers::set_custom_providers`), so
  they appear alongside the built-ins in the TUI `/provider` selector, the
  onboarding wizard, the desktop settings/model bar, tab completion, and
  every credential/base-URL resolution path. Chat dispatch maps each custom
  provider's `api_format` (`openai` | `anthropic` | `gemini` | `xai`) onto
  the matching genai adapter, and model listing honours the format (with a
  static-list fallback when the endpoint is unreachable). New TUI commands:
  `/provider add <id> <base_url> [format=…] [key=…] [models=…] [name=…]`,
  `/provider remove <id>`, `/provider list`.
- **Joshua local inference engine.** [Joshua](https://github.com/rexlunae/joshua)
  (pure-Rust GGUF server) is now a first-class engine and provider:
  detect/install (`cargo install`), start/stop (`joshua serve --model … --addr
  127.0.0.1:8331`), GGUF model scan of `~/.rustyclaw/models/joshua` (or the
  configured `models_dir`), Hugging Face pulls (GGUF + `tokenizer.json`), and
  load/unload by restarting the single-model server. `EngineConfig` gains a
  `default_model` field for single-model-per-process engines, and engine
  auto-start (`engine_service_defs`) resolves the GGUF to serve.
- **`/engines` panel in the TUI.** The previously stubbed engines dialog is
  now wired end-to-end: `/engines` opens a live panel showing each engine's
  install/run state, endpoint, and models; ↑/↓ selects an engine, Enter lists
  its models, `s` starts/stops, `i` installs, `r` refreshes, and pull progress
  renders in-panel. Subcommands: `/engines start|stop|install <engine>`,
  `/engines models <engine>`, `/engines pull <engine> <model>`,
  `/engines load|unload|remove <engine> <model>`.

### Fixed

- **Clients stopped being asked for their 2FA code.** Whether to challenge a
  connecting client read `config.totp_enabled` alone — what onboarding last
  recorded, not what 2FA is. The enrolled secret lives in the vault, and
  `totp_enabled` is not part of the boot slice, so a config replaced by
  defaults (a hand-edited file, or `Config::load` quarantining a torn one)
  dropped the flag while the vault kept the secret: the gateway served every
  connection with no challenge, no console line and no log event, and the
  owner had no way to tell from the client that 2FA had stopped applying.
  The same flag also admits *unpaired* SSH keys, on the understanding that
  the code challenge is there to catch them — so the two together opened a
  gateway to any key. The decision is now
  `SecretsManager::totp_required`, asked of the vault by both, the same rule
  the secret viewer's step-up check already used: an enrolled secret means a
  code is required whatever the config says; a readable vault with no secret
  means no challenge (there would be nothing to check it against); and a
  locked or not-yet-written vault falls back to the config flag. Onboarding
  reconciles the flag with the vault on the way through, so a drifted config
  is repaired rather than worked around.

- **A gateway behind OpenSSH hung up instead of asking for a code.** Under
  `--ssh-stdio` sshd owns the socket, so the gateway sees a pipe with no
  peer address — and the TOTP path needed one for rate limiting, closing the
  connection before the challenge went out. That reached the user as a
  client that never prompts and a connection that dies on its own. Peerless
  transports are now challenged like any other and share one rate-limit
  bucket under an address no peer can present. `rustyclaw gateway reload`
  had the mirror-image bug: it decided from its own `totp_enabled` whether
  to expect a challenge (and one branch had the test inverted), so it could
  sit waiting on a code it never prompted for. It now answers whatever the
  gateway asks for.

- **Typing in the desktop client was jerky, and kept becoming jerky again.**
  Every text field is a controlled input — a keystroke is copied into a Rust
  signal, re-rendered, and written back onto the field — so any render that is
  slow or scales with the conversation is felt directly as lag or as swallowed
  characters. Two paths did both. Building the transcript re-ran an HTML parse
  and a CommonMark parse over *every* message in the thread, and it runs inside
  the component that owns the composer's draft text, so a long conversation was
  re-sanitised once per character typed; sanitised markdown is now cached by
  source, leaving a keystroke parsing nothing and a streaming flush re-parsing
  only the bubble that changed. The dialogs were rendered by a plain function
  call inlined into `App`'s reactive scope while their draft signals (the TOTP
  code, the agent name) were declared in `App`, so one digit of a TOTP code
  re-cloned the whole message list into the chat's props and rebuilt the
  sidebar tree; the dialogs are now a component with their own scope and own
  their draft text, which `AppSignals` no longer carries. Neither fix removes a
  keyboard handler — the `onkeydown` shortcuts are one key comparison each and
  were never the problem — but every one of them is now documented with its
  reason in `input_latency.rs`, and a test fails on an undocumented one, on a
  global key listener, on dialogs moving back into `App`'s scope, and on a
  keystroke or a streaming flush re-parsing markdown it does not need to.
  `rustyclaw-desktop` is now in the CI unit-test job — it was in none, which is
  why this kept coming back unnoticed. See `docs/input-latency.md`.

- **Every paired client was locked out after the gateway restarted.** The
  SSH auth check compared whole `PublicKey` structs, and that equality
  includes the key's comment — which exists only on the disk side. Pairing
  persists the key to `authorized_clients` with a comment
  (`user@rustyclaw`), while a key arriving in an SSH auth request never
  carries one (the wire blob has no comment field). So the comparison
  matched only the in-memory copy bootstrapped by the current process:
  the first restart re-read the file and rejected every paired client with
  `Permission denied (publickey)`, indefinitely, while the port still
  answered probes. The check now compares key material only. Relatedly,
  the desktop rendered only the outermost error layer — a generic
  "Gateway at … is not responding" — hiding the `Permission denied`
  underneath; it now shows the full error chain, so an auth rejection, a
  refused connection and a host-key mismatch are all distinguishable from
  a gateway that is actually down.

- **Model tuning and the provider TLS pin were erased by `boot.toml`.**
  `BootConfig::apply` rebuilt the whole `[model]` section from the boot slice,
  hard-coding `tls_ca_cert`, `reasoning_effort`, `max_tokens`, `temperature`
  and `token_budget` to `None`. That cost nothing while no install had a
  `boot.toml`; creating one for every install would have made it universal —
  a user's tuning silently gone on the next start, made permanent by the
  following save, and the trust-anchor pin from #234 never installed, so
  provider traffic fell back to the system trust store while the operator
  believed it was pinned. `apply` now overrides only what the boot slice
  actually owns. The tuning fields describe how requests should be shaped and
  mean the same thing under any provider, so they always survive; `base_url`
  and `tls_ca_cert` name one provider's endpoint, so they survive while the
  provider is unchanged and are dropped on a switch — and dropping a pin now
  says so on stderr rather than un-pinning in silence.

- **Saving a provider, model, workspace or SSH bind change was silently
  reverted on the next start once `boot.toml` existed.** `boot.toml` wins over
  `config.toml` for the fields it carries, but nothing kept it in sync, so
  `Config::save` wrote a change that the next `Config::load` overwrote from a
  stale boot file. `rustyclaw config set model.provider anthropic` printed
  `✓ Set model.provider = anthropic`, `config.toml` genuinely said
  `anthropic`, and the next boot read `openai` — the same for `/model`, the
  gateway's admin model switch, the workspace path and the SSH bind. `save`
  now writes the boot slice through to `boot.toml`, anchored at
  `settings_dir` so it lands where `load` looks for it — and *removes* it when
  the config no longer has any boot-critical fields at all. Skipping an empty
  slice (right when creating the file, wrong when maintaining it) left the old
  mirror on disk after `config unset model`, so the next start quietly
  reinstated the provider the user had just removed. The mirror is touched
  only when the file being read or written *is* this install's own config,
  in both directions. Anchoring on `settings_dir` alone let any config reach
  into whatever state directory it happened to name: on the save side, since
  `Config::default()` names `~/.rustyclaw`, `cargo test` deleted a
  developer's real `boot.toml` — and passed; on the load side, one command
  against a side file (`--config /tmp/experiment.toml` naming the real
  settings dir) planted that file's provider and model in the install's
  `boot.toml`, and because `boot.toml` outranks `config.toml`, every later
  normal start read the planted values. Reading someone else's config file
  does not reconfigure the machine.

- **A failed gateway start deleted the running gateway's PID file, leaving it
  unstoppable.** The PID file is written before anything is bound and was
  removed unconditionally on the way out, so a second gateway started against
  an occupied port overwrote the record on the way in and deleted it on the
  way out: `gateway status` reported `stopped` while the real gateway kept
  serving connections, and `gateway stop` could no longer reach it. Removal
  now happens only while the file still names the exiting process, and the
  gateway refuses to start at all when the record names another live process
  — the same refusal `rustyclaw gateway start` has always made, now also made
  by the binary run directly, and made *before* any managed service is
  started. The PID file is not touched at all under `--ssh-stdio`, where one
  instance runs per connection and the record belongs to the daemon.

- **A gateway that could not bind its SSH port reported itself as listening
  and then accepted nothing for the rest of its life.** The bind happened
  inside a detached task, *after* `SshServer::listen` had already logged
  "SSH server listening" and returned `Ok(())` — and `main` had printed
  "Gateway listening on SSH …" before that, from its own second resolution
  of the address. So a port already in use (a second gateway, or a stale one
  the PID file had lost track of) produced a process that announced itself
  as listening three times over, wrote a PID file, reported `running` from
  `gateway status`, and refused every connection. The only trace was one
  `ERROR` line from the detached task, logged beneath the three cheerful
  ones. The socket is now bound before `listen` returns, so the failure is
  the startup error it always was; the address announced is the one actually
  bound (including the port the kernel picks for `:0`); and if the accept
  loop later stops — russh returns from it on the first accept error —
  `accept` reports that instead of parking forever on a queue nothing will
  feed, so the gateway exits rather than lingering unreachable. The accept
  loop used to log-and-continue on that error, spinning at full CPU. Listener
  setup and the accept loop now share one exit path, so a failed bind runs the
  same managed-service shutdown a cancelled one does: the `ServiceManager`
  lives in a `'static` runtime context that is never dropped, so an early
  return would have left every auto-started `[services.*]` process — a local
  inference server, say — running with nobody managing it, and `kill_on_drop`
  cannot fire on a child whose manager outlives the process. `--ssh-stdio` returns from
  the middle of the function and skipped the shutdown too, so every SSH
  connection left a full set of service processes behind; its early return now
  runs the same shutdown. Auto-start still happens in that mode — the
  documented OpenSSH-subsystem deployment has no standalone daemon, so the
  per-connection instance is the only thing that can start them — and
  `stop_all` only stops what its own manager started, so an instance cleans up
  its own children without reaping another session's.

- **The gateway never asked for the vault passphrase on an encrypted setup
  whose config flag had gone false.** The decision to prompt read only
  `config.secrets_password_protected`, which records what onboarding chose
  rather than what is on disk. Whenever the config was replaced by defaults
  — a hand-edited file, or `Config::load` quarantining a torn one — the flag
  went false while `credentials/secrets.json` stayed encrypted, and the
  gateway started with no prompt, no console line and no log event, every
  secret in it unreachable. The rule now lives once, as
  `SecretsManager::requires_password`: a vault file with no key file beside
  it can only be opened with a password. The gateway, `rustyclaw gateway
  start`/`restart`, and onboarding all ask it (still OR-ed with the config
  flag, which alone means "password" for a vault not yet written). A prompt
  that cannot be read, or is answered empty, now starts the vault *locked*
  for a client to unlock rather than opening it with an empty password —
  `.unwrap_or_default()` used to make those indistinguishable.

- **The gateway installed no tracing subscriber, so every log line it
  emitted was discarded.** `tracing` is a no-op facade until something is
  listening, and nothing ever was: the daemon's `error!`, `warn!`, `info!`
  and `trace!` calls — the whole crate's worth — went nowhere, `RUST_LOG`
  was never consulted because there was no filter to consult, and the only
  evidence a live session left behind was the client's protocol event log.
  The gateway now installs a subscriber as its first act, before the vault
  is opened or a port is bound. It logs to stderr, which a foreground run
  shows on the terminal and `rustyclaw gateway start` already redirects
  into `<settings_dir>/logs/gateway.log`; under `--ssh-stdio` it writes to
  that file directly, since there stderr is an ssh channel the client only
  drains when the session ends — a chatty filter would fill the pipe and
  wedge the connection. `RUSTYCLAW_LOG_FILE` overrides the destination in
  either mode.

- **`RUSTYCLAW_LOG`/`RUST_LOG` now outrank the filter a binary configures
  for itself**, rather than being consulted only by the callers that
  happened to build their config with `LogConfig::from_env`. A directive
  that does not parse falls through to the next candidate instead of
  taking the process's logging down with it. Log output is also colourless
  when the stream is not a terminal, so a redirected `gateway.log` no
  longer arrives full of escape codes.

- **`MessengerConfig` no longer prints credentials in `Debug` output.** The
  derived impl put live bot tokens and passwords into the log line emitted
  when a messenger fails to initialize. Secrets are now redacted while the
  non-secret fields — the ones that make a failure diagnosable — are kept.

- **The `freenet`, `river`, and `atlas` tools had no panel category**, which
  tripped the gateway's `tool_categories_cover_registry` test.

- **A turn held the whole gateway connection, so nothing else could be
  answered until it finished.** `handle_chat_frame` was awaited inline in
  the connection loop, so every other client frame — thread switch, history
  request, project change, model switch — sat in the queue behind the
  running turn. Switching threads mid-turn moved the sidebar highlight and
  showed cached messages, but the authoritative history only arrived once
  the model was done, and a turn parked on an `ask_user` question could
  hold the connection for the full five-minute wait. Each turn now runs in
  its own task, writing through the `ChannelSink`/`ActiveTasks` plumbing
  that was already built for it (and had been left half-wired), while the
  connection loop goes straight back to serving frames. The thread manager
  moved behind a shared mutex to make that possible; every lock scope is a
  single operation, and the two client-bound thread senders snapshot under
  the lock and release it before writing, since holding it across a frame
  write would deadlock the turn against the connection loop as soon as the
  frame channel filled. A second message to a thread that is already
  working is refused with a note rather than interleaved into an
  unreadable transcript, a turn still running when the client disconnects
  is cancelled and given ten seconds to flush instead of being orphaned,
  and the cancel flag is now reset when a turn *starts* rather than on
  every inbound frame — which previously threw away a Stop the moment any
  other command arrived.

- **An agent question was inline in looks only — everything else stopped
  until you answered it.** The desktop's `ask_user` card rendered in the
  chat stream, but it replaced the composer, which took the **Stop**
  button, model picker and directory picker down with it; it stayed on
  screen no matter which thread you switched to; and the gateway's wait
  for an answer ignored the cancel flag, so Stop could not end a turn
  parked on a question — the only ways out were answering, dismissing, or
  the five-minute timeout. The composer now stays mounted with the
  question as a band beneath it, so Stop stays one click away; the
  question belongs to the thread that asked it (switch away and it is
  parked, switch back and it returns); pressing Stop ends the wait
  gateway-side and the tool reports back that the user stopped the turn;
  and the card is retired by the `ask_user` tool's own result, so a
  question abandoned by a cancel or a timeout does not linger. Keyboard
  handling is on the card as a whole rather than only its text field:
  **Enter** submits from anywhere in it — including select, multi-select
  and multi-field forms, which had no Enter handler at all — **Esc**
  dismisses, and ↑/↓ move the selection in a single-select. Rows of
  buttons stop Enter before it reaches that handler, so tabbing to
  **Dismiss** or **No** and pressing Enter does what the button says
  instead of submitting the default answer. The card also takes focus
  explicitly when it mounts, since a webview ignores `autofocus` on nodes
  inserted after page load, which is every one of these cards.

- **Creating a project meant typing a path from memory, and failing at it
  on macOS.** The New Project dialog had no folder picker (unlike the
  Edit Project dialog) and offered `/home/you/code/my-project` as its
  example, so a macOS user following that shape landed on `/home/...` —
  a path owned by the system automounter, where `mkdir` fails with
  `Operation not supported (os error 45)` and the gateway reported
  exactly that and nothing else. The dialog now starts with the path
  field on the user's home directory, has the same **Browse…** native
  folder picker as the edit dialogs (filling in the project name from the
  chosen folder when the name field is still empty), and every
  directory placeholder is derived from the real home directory instead
  of a hard-coded Linux path. Gateway-side, project and thread
  directories now go through one `prepare_workspace_dir` helper that
  expands a leading `~` (previously stored verbatim, which would create a
  directory literally named `~` beside the gateway's working directory)
  and explains a failure in terms of what to do about it — naming the
  `/Users` path that would have worked for a macOS `/home` path, the
  closest existing ancestor, the unwritable directory, or the file
  standing where a directory should be. `expand_tilde` in core also
  stopped expanding `~alice/notes` into `$HOME/alice/notes`.

- **Thread history that never made it onto the screen.** Opening the
  desktop client could show nothing but system notices while the sidebar
  correctly reported each thread's message count — the messages were on
  disk and on the wire, but no client ever displayed them. Clients drive
  history loading off the thread list's `foreground_id`, and the gateway
  could report `None` for it indefinitely: closing the foreground thread
  left the manager with no foreground, as did the "background the current
  thread" sentinel, and both states are persisted to `threads.json`. With
  no foreground, the client had no thread to request history for, the
  gateway skipped its unprompted history push, and the guard in
  `apply_thread_history` rejected any snapshot that did arrive.
  `ThreadManager` now keeps the invariant that a manager holding threads
  always names a foreground — electing the most recently active chat
  thread on load and when the foreground thread is closed — and closing a
  thread now pushes the new foreground's history so the transcript follows
  the sidebar. The desktop no longer discards authoritative history: a
  snapshot arriving before the client knows its foreground thread is
  displayed rather than cached and forgotten, and a thread list that names
  a foreground shows the snapshot that already arrived for it.
- **New threads overwriting old ones after a restart.** Thread ids are
  minted from a process-global counter that starts at 1, but they are
  persisted and restored on the next run. A restarted gateway therefore
  handed the first new thread an id that a restored thread already owned,
  replacing it in the map — silently destroying that thread's entire
  message history. Loading now reserves the counter above every restored
  id.
- **Two copies of the wire→transcript conversion in the desktop client**
  had drifted apart, mapping unrecognised message roles differently
  (`ThreadMessages` rendered them as inline `ℹ️` notices,
  `ThreadHistoryReply` as neutral system lines). Both history frames now
  share one conversion.
- **TUI commands that printed success and did nothing.** `/clear` now
  actually clears the display (and says thread history is unaffected
  rather than claiming memory was cleared); `/gateway` reports the real
  connection status; `/gateway start|stop|restart` and `/download`
  no longer pretend — they explain what to use instead. Multi-select
  agent prompts only ever recorded one selection: Space now toggles
  per-option checkboxes (seeded from prompt defaults) and Enter submits
  all checked options. `/help` documents the previously hidden keyboard
  shortcuts, `/quit`, and the full `/engines` subcommand list; the
  unimplemented `/analytics`, `/logs`, and `/approvals` commands no
  longer tab-complete.
- Switching providers no longer carries a stale `base_url` override from the
  previous provider into the new selection (it is kept only when the new
  provider has no catalogue URL, e.g. `custom` / `copilot-proxy`).

### Changed

- **The interactive tool loop is paced by rate, not stopped at a count.**
  Long-running agent tasks were dying at an arbitrary ceiling: after 500
  tool rounds the turn was killed with "Safety limit reached (500 tool
  rounds) — stopping to prevent infinite loop", regardless of whether it
  was looping or three hours into legitimate work — while a genuinely
  runaway loop was free to burn all 500 rounds as fast as the provider
  answered before the cap ever engaged. The absolute cap is gone. In its
  place the loop is paced to `tool_limits.max_rounds_per_minute` (default
  60, `0` disables): a turn over the rate *waits* for the sliding
  one-minute window to open — with an info notice so the client can see
  the pacing — and then continues; it is never stopped. Cancellation stays
  responsive while paced. The detectors for loops that really are stuck
  are unchanged: three consecutive rounds of all-failed tool calls, the
  auto-continue cap, and user cancel. Headless loops (cron, triggers,
  messengers, spawned subagents) keep their small round caps — they run
  unattended, with nobody watching to cancel.

- **Structured-error follow-up: restored ~100 context sites lost to the
  revert probe, dropped redundant conversions.** The pass that introduced
  `ToolError::Context` had collaterally reverted convertible call sites
  back to `format!` flattening (an earlier `cargo fix` had stripped
  then-unused `ToolError` imports in 21 files, so re-conversion attempts
  failed to resolve the type and were misclassified as non-convertible).
  Those imports are repaired and the sites re-converted — the tool layer
  now preserves typed sources at 140 context sites, with `format!` left
  only on genuine third-party leaf errors (chromiumoxide, zip, image, …).
  Also removed the extraneous conversions the same pass left behind:
  tail `.map_err(ToolError::from)` no-ops replaced by constructing
  `ToolError::msg` / `missing_param` directly, and gateway handler
  imports trimmed to what they use.

- **Structured-error preservation pass over the tool layer.**
  `ToolError` gains a `Context` variant plus `ToolError::context(ctx, e)`:
  it renders identically to the previous `format!("ctx: {e}")` flattening
  but keeps the typed error reachable via `source()`. 37 convertible
  context sites now preserve their sources; `format!` remains only for
  third-party leaf errors with no `ToolError` conversion. A new `Ssrf`
  variant propagates `SsrfError` verdicts through `web_fetch` untouched,
  and the gateway model/task handlers use plain `?` instead of
  `.map_err(|e| e.to_string())` for registry/service/task errors.
  Unit tests assert the source chain survives `Context` wrapping.

- **AI-tool layer moved to typed errors (`ToolError` / `ToolResult`).**
  All `exec_*` tool implementations (~45 files in `core/src/tools/**` and
  the gateway tool handlers) now return `ToolResult` instead of
  `Result<String, String>`. `ToolError`'s `Display` is the exact
  model-facing message; per-module typed errors (`SandboxError`,
  `ProcessError`, `TaskError`, `ServiceError`, `RegistryError`,
  `CronError`, `ConsolidationError`, `MemoryIndexError`, `SessionError`,
  `SwarmError`, `SteelMemoryError`, `io`/`serde_json`/`reqwest`) propagate
  into it with plain `?`, bespoke messages route through `ToolError::Msg`,
  and the gateway tool executor is the single point where the error is
  flattened to the model-payload string. Also typed in the same pass: the
  tool-call rate limiter (`RateLimitError`), `read_memory_file`
  (`MemoryIndexError::{InvalidPath, NotFound}`), `SubconsciousError` and
  `SyncError` (now enums preserving `anyhow` cause chains), and the
  subtask closure contract (`Result<T, SubtaskError>`). The dead
  `tools::{ToolCall, ToolResult}` wire structs (zero users) were removed,
  and STYLE_GUIDE §5 now describes the `ToolError` pattern instead of the
  `Result<String, String>` exception.

- **Completed the typed-error migration started in #303.** Remaining
  internal `Result<_, String>` plumbing now uses per-module `thiserror`
  enums, with strings only at the documented display boundaries:
  `SteelMemoryError` (steel_memory.rs — audit follow-up #1),
  `SandboxError` (sandbox + command-safety helpers — audit follow-up #2,
  policy verdicts distinguishable from execution failures),
  `TaskError` (tasks/manager.rs), `SubtaskError` (threads/subtask.rs —
  replaces the `"Cancelled"` sentinel-string comparison), `ReceiptError`
  (protocols/receipt.rs), `CustomProviderError` (providers/custom.rs),
  `MissingRequestField` (gateway resolve_request),
  `ProcessManager::spawn` returns `ProcessError`, the SSH bare-frame
  fallback returns `FrameCodecError`, and the desktop swarm helpers
  propagate `SwarmError` via `anyhow` instead of pre-flattening.
  `docs/RUST_IDIOMS_AUDIT.md` follow-ups #1 and #2 are marked fixed.
- **Provider backend migrated to the `genai` crate.** The gateway's hand-rolled
  OpenAI / Anthropic / Google HTTP clients
  (`rustyclaw-gateway/src/providers/{openai,anthropic,google}.rs`) are replaced
  by a single [`genai`](https://crates.io/crates/genai)-backed dispatch in
  **`rustyclaw-core`** (`providers/genai_backend.rs`). It lives in core so the
  gateway and the client crates share one genai instance. Request building, tool
  calling, and SSE streaming (including Anthropic extended-thinking deltas) are
  now handled by genai; RustyClaw still owns provider selection, credentials /
  Copilot session tokens, and the binary streaming frame protocol. Each provider
  id maps onto a genai adapter; all OpenAI-compatible providers (OpenRouter,
  Ollama, LM Studio, exo, OpenCode, GitHub Copilot, custom) use the OpenAI
  adapter at their configured base URL. The gateway's
  `providers::call_{openai,anthropic,google}_with_tools` re-export the core
  implementation, so dispatch / messenger / thread / compaction call sites are
  unchanged.

### Notes

- Tool-loop continuation messages now use a single provider-agnostic canonical
  encoding (`providers::encode_assistant_message` / `encode_tool_result`)
  instead of per-provider JSON shapes.
- The previous automatic fallback to the OpenAI *Responses API* (for models that
  reject `/chat/completions`) is not reproduced; genai selects the Responses API
  adapter from the model name instead.

## [0.1.0] - 2026-02-12

### 🎉 Initial Release - Full OpenClaw Parity

This release achieves complete feature parity with OpenClaw's agentic capabilities.

### Added

#### Tools (30 total)
- **File tools**: read_file, write_file, edit_file, list_directory, search_files, find_files
- **Runtime tools**: execute_command, process (background management)
- **Web tools**: web_fetch (URL content extraction), web_search (Brave Search API)
- **Memory tools**: memory_search (BM25 keyword search), memory_get (snippet retrieval)
- **Scheduling**: cron (at, every, cron expressions)
- **Session tools**: sessions_list, sessions_spawn, sessions_send, sessions_history, session_status, agents_list
- **Editing**: apply_patch (multi-hunk unified diff)
- **Secrets tools**: secrets_list, secrets_get, secrets_store
- **System tools**: gateway (config/restart/update), message (send/broadcast), tts
- **Media**: image (vision model analysis)
- **Devices**: nodes (camera, screen, location, remote exec)
- **Browser**: browser (Playwright/CDP automation)
- **Canvas**: canvas (A2UI presentation)

#### Skills System
- SKILL.md parsing with YAML frontmatter
- Gate checking: bins, anyBins, env, config, os
- Prompt context injection for eligible skills
- `{baseDir}` placeholder substitution
- Directory precedence: workspace > local > bundled

#### Messenger Backends
- WebhookMessenger - POST to any URL
- ConsoleMessenger - stdout for testing
- DiscordMessenger - bot API integration
- TelegramMessenger - bot API integration

#### Provider Streaming
- OpenAI SSE streaming with tool call support
- Anthropic SSE streaming with content blocks
- mpsc channel-based chunk delivery

#### Gateway
- WebSocket server with ping/pong keepalive
- TOTP 2FA authentication
- Rate limiting and lockout
- Multi-provider support (OpenAI, Anthropic, Google, GitHub Copilot, xAI, Ollama, OpenRouter)
- Context compaction at 75% window

#### TUI
- Slash commands: /help, /clear, /provider, /model, /gateway, /secrets, /skills, /status, /quit
- Tab completion
- Pane navigation (ESC/TAB)
- Message scrolling

#### Secrets Vault
- AES-256 encrypted storage
- Access policies (Always, WithAuth, SkillOnly, Never)
- TOTP 2FA protection
- Rate limiting and lockout

#### Testing
- 152+ unit tests
- 200+ integration tests
- CLI conformance tests
- Gateway protocol tests
- Skill execution tests
- Tool execution tests
- Exit code tests
- Golden file tests
- Streaming tests

#### CLI Commands
- setup, onboard, configure
- config get/set/unset
- doctor --repair
- tui
- command (one-shot)
- status
- gateway start/stop/restart/status
- skills list/enable/disable

### Project Logo
- Half gear / half lobster claw design (logo.svg)

---

## Future Roadmap

### Planned for 0.2.0
- [ ] Full Playwright/CDP browser implementation
- [ ] Real vision model integration
- [ ] Real TTS service integration (ElevenLabs)
- [ ] Slack messenger backend
- [ ] WhatsApp messenger backend
- [ ] Signal messenger backend
- [ ] Google Gemini streaming

### Planned for 0.3.0
- [ ] Plugin system
- [ ] Tool profiles and policies
- [ ] Remote node execution
- [ ] macOS app bundle
