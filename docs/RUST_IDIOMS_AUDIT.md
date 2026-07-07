# Rust Idioms Audit

An audit of the workspace against the conventions in [`STYLE_GUIDE.md`](../STYLE_GUIDE.md),
focused on five dimensions:

1. Errors propagated with `Result` and typed errors, not flattened into `String`s
   except at the boundary that displays them.
2. Module size and cohesion (one main topic per module).
3. Enums instead of strings for closed value sets.
4. `From`/`Into` impls instead of ad-hoc conversion functions.
5. Traits for abstractions.

Items marked **[fixed]** were addressed in the PR that added this document;
the rest are recorded here as prioritized follow-ups.

---

## 1. Error handling

The codebase has two documented error strategies (`rustyclaw-core/src/error.rs`):
typed errors for internal logic, and `Result<String, String>` at the AI-tool
boundary, where the error string is the payload sent back to the model. The
tool boundary is a legitimate display boundary — flattening a typed error with
`.to_string()` there is correct. The audit therefore focused on *internal*
plumbing that used `Result<_, String>` or flattened errors mid-propagation.

### Fixed

- **[fixed]** `error.rs` carried four conversion helpers
  (`anyhow_to_tool_err`, `anyhow_to_tool_result`, `tool_err_to_anyhow`,
  `tool_to_anyhow_result`) with zero callers — removed; the module doc now
  describes the actual strategy (per-module `thiserror` enums internally,
  strings only at the tool/model boundary, `anyhow` in binaries).
- **[fixed]** New per-module `thiserror` enums replace `Result<_, String>` on
  internal APIs, with `#[from]` conversions so `?` propagates sources:
  `SsrfError` (`security/ssrf.rs` — callers can now distinguish a security
  `Blocked` verdict from a transient DNS failure), `CronError` (`cron.rs`),
  `ServiceError` (`services/manager.rs`), `SwarmError` (`swarm/manager.rs`),
  `SessionError` (`sessions.rs`), `MemoryIndexError` (`memory.rs`),
  `ConsolidationError` (`memory_consolidation.rs`), `RegistryError`
  (`models/registry.rs`), `ProcessError` (`process_manager.rs`).
- **[fixed]** `tokenjuice/src/compile.rs` flattened regex-build errors into
  `format!` strings even though `CompileError` existed in the same file — now a
  `CompileError::Regex` variant.
- **[fixed]** Frame codec (`gateway/protocol/frames/codec.rs`) returned
  `Result<_, String>`; now returns a typed `FrameCodecError`
  (`TooLarge`/`Encode`/`Decode`).
- **[fixed]** Swallowed errors: `steel_memory.rs` ignored `create_dir_all`
  failures for the embedding cache; `cron.rs` silently dropped corrupt
  run-history lines (`filter_map(.. .ok())`); `services/manager.rs` ignored
  child-kill failures on stop; `memory_consolidation.rs` degraded a size-limit
  breach to `eprintln!` in library code; the gateway silently fell back to
  `SandboxMode::Auto` on an invalid `sandbox.mode` config value. All now
  propagate or `tracing::warn!`.

### Follow-ups (prioritized)

1. **[fixed]** ~~`steel_memory.rs`~~ — now uses `SteelMemoryError { Storage,
   Embedding, KnowledgeGraph, Palace, Io, ModelNotInitialized, TaskPanicked }`
   with the underlying `anyhow::Error` chains preserved per operation, plus a
   `run_blocking` helper that replaces the repeated triple-match on
   `spawn_blocking` results. Flattening now happens only at the memory-tool
   boundary and in `MemoryTreeError::Summarizer` (a foreign string variant).
2. **[fixed]** ~~`sandbox/platform.rs`~~ — `SandboxError` in `sandbox/mod.rs`
   covers the policy verdicts (`ReadDenied`, `NotAllowed`, `ExecDenied`,
   `Command*`, `SymlinkRace`) separately from execution failures
   (`Spawn(io)`, `Landlock`/`LandlockPath` with real `#[source]`s,
   `Unsupported`). `validate_path`, `run_sandboxed`, `Sandbox::{init,
   check_path, run_command}`, and the `tools/helpers.rs` security checks
   (`validate_command_safe`, `resolve_path_no_race`,
   `run_sandboxed_command`) all return it; `ServiceError::CommandRejected`
   now carries it as a typed source.
3. **[fixed]** ~~Gateway parse helpers~~ — the whole AI-tool layer
   (all `exec_*` implementations in `core/src/tools/**` and the gateway
   tool handlers, parse helpers included) now returns `ToolResult`
   (`Result<String, ToolError>`). `ToolError` carries `#[from]`
   conversions for the per-module enums (`SandboxError`, `ProcessError`,
   `TaskError`, `ServiceError`, `RegistryError`, `CronError`,
   `ConsolidationError`, `MemoryIndexError`, `SessionError`, `SwarmError`,
   `SteelMemoryError`, plus `io`/`serde_json`/`reqwest`) and a `Msg`
   variant for bespoke messages (reached via `From<String>`, keeping
   `.map_err(|e| format!("context: {e}"))?` as the context idiom).
   The gateway's tool executor is the single flatten point. The tool-call
   rate limiter returns a typed `RateLimitError`.
4. **[fixed]** ~~`SubconsciousError(String)` / `SyncError(String)`~~ —
   both are now enums (`Message(String)` for implementation-described
   failures, `Source(anyhow::Error)` preserving cause chains), and the
   subtask spawn functions' closure contract is `Result<T, SubtaskError>`
   instead of `Result<T, String>`.
5. **`gateway/errors.rs`** is the reference implementation for the
   display-boundary pattern (typed kind + source, formatted only in
   `user_message`) — new error types should imitate it.

Addressed in the same pass (post-#303 stragglers): `TaskError`
(`tasks/manager.rs`), `ProcessManager::spawn{,_with_owner}` now return
`ProcessError`, `SubtaskError` replaces the `"Cancelled"` sentinel string in
`threads/subtask.rs`, `ReceiptError` (`protocols/receipt.rs`),
`CustomProviderError` (`providers/custom.rs`), `MissingRequestField`
(`gateway/providers/mod.rs` `resolve_request`), the SSH bare-frame fallback
returns `FrameCodecError` instead of `String`, and the desktop swarm helpers
propagate `SwarmError` through `anyhow` instead of pre-flattening.

## 2. Module size & cohesion

Verdicts from the audit of the ~16 largest files:

- **[fixed]** `gateway/protocol/frames.rs` (1489 lines) mixed three topics:
  payload enums, a ~20-struct DTO catalog, and the bincode codec. Split into
  `frames/dto.rs` and `frames/codec.rs` with re-exports, keeping external
  paths stable.
- **`rustyclaw-gateway/src/server.rs`** — `handle_connection` is a single
  ~830-line function mixing TOTP auth + rate limiting, session exchange,
  bootstrap, reader-task spawning, and the dispatch loop. Proposed split:
  `server/{auth,bootstrap,session,reader}.rs`; all internal, low risk. Largest
  remaining win.
- **`rustyclaw-gateway/src/dispatch.rs`** — one topic (the agent loop) but
  `dispatch_text_message` is ~800 lines; extract phase helpers
  (`refresh_bearer`, `maybe_flush_memory`, `run_tool_round`).
- **`rustyclaw-desktop/src/app/mod.rs`** — gateway auto-connect and the event
  pump coroutine (network IO) live inside the `App()` UI component; extract
  into hooks (`app/effects.rs`).
- **TUI `events.rs` / `keyboard_normal.rs`** — single giant matches over a
  destructured 30-signal `state::Ui` bundle; splitting the bundle would shrink
  all three files that destructure it.
- **Long but cohesive — leave as-is:** `tools/definitions.rs`,
  `tools/params/{mod,ext}.rs` (flat schema/registry tables),
  `sandbox/platform.rs` (N platform backends behind `#[cfg]`),
  `cli/main.rs` (clap dispatch), `skills/mod.rs`, `onboard/lib.rs`.

## 3. Strings → enums

`strum` was already a workspace dependency but previously unused.

### Fixed

- **[fixed]** Protocol action verbs, formerly `action: String` fields with
  `_ => Err("Unknown action")` fallthroughs, are now enums with compile-time
  exhaustiveness: `CronActionKind`, `ChannelPairActionKind`,
  `EngineActionKind`, `ModelActionKind` (`gateway/protocol/frames.rs`,
  threaded through `GatewayCommand`, the engine handler, and the desktop
  engines panel).
- **[fixed]** `SshGatewayConfig.mode: String` → `SshMode { Standalone,
  Subsystem }` (serde-lowercase keeps existing config files working; also
  fixes `Default` yielding an empty string instead of "standalone").
- **[fixed]** `AccessPolicy` gained `from_badge`/`cycled` so UI layers can
  stop hand-rolling `"OPEN" → "ASK" → "AUTH" → "SKILL"` rotation tables.

### Follow-ups (prioritized)

1. **`role: String`** (`"user" | "assistant" | "system" | "tool"`) — defined
   in `sessions.rs`, `streaming.rs`, `protocol/types.rs`, `view/memory.rs`,
   `memory-tree`; compared as literals across ~23 files. The single
   highest-value enum conversion (`MessageRole`), but also the widest blast
   radius — do it as a dedicated change. Note `rustyclaw-view` already has a
   `MessageRole` enum that the wire layer bypasses.
2. **`SecretsSetPolicy` policy vocabulary** — three string vocabularies exist
   for one concept: UI badges (`"OPEN"`), wire strings (`"ask"`,
   `"skill_only"`), and `AccessPolicy` serde names (`"with_approval"`). The
   wire field should carry `AccessPolicy` directly, folding the `skills` field
   into `SkillOnly`.
3. **`SandboxConfig.mode: String`** — `SandboxMode` exists with alias-aware
   `FromStr`; the config field should be typed (custom `Deserialize` via
   `FromStr` to keep aliases like `"bubblewrap"`).
4. **`messenger_type`** — two big matches in `messenger_handler/builders.rs`
   over a closed set of ~10 messenger names.
5. **Status DTOs** — `McpServerDto.status`, `ServiceInfoDto.status`,
   swarm agent status/role in `rustyclaw-view` (each matched in 2–3 places,
   capitalization-sensitive).
6. **`provider: String`** — the value set is open (`custom` + arbitrary
   base URLs) so the field stays a string, but the scattered
   `provider == "anthropic"` special-cases should route through one place
   (partially addressed by `providers::call_with_tools`, see §5).
7. Smaller closed sets: `UsageStatsRequest.period` (`day|week|month|all`),
   observability `direction` (`inbound|outbound`), view `auth_hint`
   (`apikey|deviceflow|none`), `LogsRequest.source` (closed prefix + open
   service-name tail → enum with a `Service(String)` variant).

## 4. `From`/`Into` conversions

The server side already used `From` for DTO conversions
(`frames.rs`: `From<EngineCaps>`, `From<LocalModel>`, `From<ServiceInfo>`);
the client/view side hand-rolled the mirror-image conversions.

### Fixed

- **[fixed]** The `rustyclaw-view` `from_dto` cluster (~12 single-argument
  constructors in `analytics`, `media`, `tools_config`, `memory`, `channels`,
  `approvals`, `mcp`, `cron`) replaced with `impl From<&Dto>`.
- **[fixed]** `SecretInfoData::{from_entry_info, from_dto}` duplicated the
  same five-field map from two sources — now one `From<&SecretEntryInfo>`
  plus delegation through the existing `From<SecretEntryDto>`.
- **[fixed]** `dto_to_service_info` (a free function in `rustyclaw-desktop`,
  an orphan-rule workaround) moved into `rustyclaw-view` as
  `From<ServiceInfoDto> for ServiceInfoData`, completing the round-trip with
  the existing `From<ServiceInfo> for ServiceInfoDto`.
- **[fixed]** `DisplayMessageData::from_chat_message` →
  `From<&ChatMessage>`; `TaskIcon::from_status` → `From<&TaskStatus>`;
  `TaskIndicator::from_task` → `From<&Task>`; `Project/Thread::to_info` →
  `From<&Project>/From<&Thread>`; genai backend
  `chat_response_to_model_response`/`to_parsed_call` → `From` impls.

### Non-goals (checked, intentionally left)

- Conversions **into** foreign types (`to_genai_chat_request`, anything
  producing `String`/`anyhow::Error`) — blocked by the orphan rule; a local
  newtype would be required and isn't worth it at current call-site counts.
- Multi-argument constructors (`MessageBubbleData::from_chat_message(msg,
  agent_name)`, `ServiceInfo::to_info(&self, name)`) — not `From`-shaped;
  tuple-based `From` impls would be strictly worse.
- `GatewayEvent::from_server_frame` returns `Option` because unmatched frames
  are legitimately ignored — more honest than `TryFrom` with a fake error.

## 5. Traits for abstractions

The workspace is already trait-rich (`RuntimeAdapter`, `Transport`/`Reader`/
`Writer`/`Acceptor`, `LocalEngine`, `Observer`, `Indexer`, `Summarizer`,
`Messenger` via chat-system). The audit found two real gaps and two
consolidations rather than ten hypothetical traits:

- **[fixed]** The 4-way copy-pasted provider dispatch (`if provider ==
  "anthropic" … else if "google" … else …` in `dispatch.rs`,
  `gateway/providers/mod.rs`, `thread_handler.rs`, `messenger_handler/mod.rs`)
  collapsed into one `providers::call_with_tools(http, req, writer)` entry
  point. New providers now have exactly one dispatch site.
- **`RuntimeAdapter` is dead code** — the single most impactful finding. The
  trait is well-designed with three impls (`NativeRuntime`, `DockerRuntime`,
  `SshRuntime`), but nothing outside `runtime/` references it: tools shell out
  via `Command::new("sh")`/`sh_async` directly, so configuring the Docker or
  SSH runtime has no effect on where tools actually run. Fix: thread
  `&dyn RuntimeAdapter` through `tools::execute_tool` and route shell
  execution through `runtime.build_shell_command(...)`.
- **Tool registry is half-abstracted** — sync tools use the data-driven
  `ToolDef` registry, but async tools live in three hand-maintained parallel
  lists (`ASYNC_NATIVE_TOOLS`, a 40-arm match ending in `unreachable!()`, and
  the summary/schema tables). A `#[async_trait] trait Tool { name, schema,
  summary, execute }` registry would remove the sync-required lists; best done
  together with the `RuntimeAdapter` wiring as one migration.
- **Sync/async twins** — nearly every executor in `tools/sysadmin/*` and
  `tools/system_tools/*` exists as `exec_X` and `exec_X_async` with duplicated
  helpers (`sh`/`sh_async`, `detect_pkg_manager`/`_async`, …). Rides on the
  two items above: keep only the async form once the unified registry exists.
- Checked and fine as-is: LLM wire formats (unified via `genai`), the
  `ClientPayload` router (arms need different state subsets), MCP client,
  secrets vault (single backends; nothing to abstract yet).
