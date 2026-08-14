# Native Plugin Architecture Plan

Status: **proposal** — no code changes yet. This document is the plan for
turning RustyClaw's current declarative panel system into a real plugin
platform: Rust dynamic libraries that register tools into the agent's toolset
and UI into the clients, loadable and unloadable at runtime, managed from the
UI, and distributed through a purpose-built registry.

Decisions already made (project owner):

- **Execution model: hybrid.** Trusted/first-party plugins load in-process in
  the gateway; repository-installed plugins run in a per-plugin host
  subprocess.
- **UI model: declarative baseline, native client dylibs allowed.** Every
  plugin's UI must work declaratively over the wire; plugins may additionally
  ship native desktop UI artifacts for richer rendering.
- **Distribution: a separate registry** purpose-built for binary artifacts
  (per-target builds, signatures) rather than extending ClawHub.

## 1. Where we are today

An audit of the existing system (August 2026), with the load-bearing facts the
design has to respect:

- **"Plugins" today are data, not code.** A plugin is a directory with
  `plugin.toml` + `state.toml` (`rustyclaw-core/src/plugins/mod.rs`). Actions
  are prose the LLM reads; clicking an action button in the desktop dock
  submits a chat message asking the agent to please run it
  (`rustyclaw-desktop/src/app/mod.rs` ~1811). There is no executor.
- **The tool registry is fully static.** `all_tools()`
  (`rustyclaw-core/src/tools/mod.rs:479`) returns `Vec<&'static ToolDef>` from
  a hardcoded list; `ToolDef.execute` is a plain `fn` pointer, so a tool
  cannot carry per-plugin state. Provider schemas derive from the same list at
  one chokepoint (`providers/genai_backend.rs:569`). Three side-tables key on
  tool *name* (`resolve_params`, `tool_summary`, `tool_category`) and
  `panel_handler::tool_toggle` rejects names not in `all_tools()`.
- **The wire protocol is bincode, positionally encoded.** Adding a field to an
  existing payload is a breaking change; new capabilities need *new frames*
  with new discriminants (`protocol/frames.rs`, `WIRE_PROTOCOL_VERSION = 3`).
  Plugin frames today: `PluginList` (80), `PluginRefresh` (81),
  `PluginsUpdate` (86). There is no install/enable/action/unload command.
- **The gateway is often remote.** Clients connect over SSH. Anything a plugin
  contributes to a client UI either travels as data or must be distributed to
  the client machine as a per-OS artifact.
- **No FFI/dynamic-loading precedent.** Zero uses of `libloading`, `cdylib`,
  `extern "C"`, or WASM anywhere in the workspace. Greenfield.
- **ClawHub (skills) is the closest repository precedent** — search/download/
  publish with device-code auth (`skills/clawhub.rs`) — but it verifies
  nothing cryptographically on install. Acceptable for Markdown skills;
  disqualifying for native code.
- **Sandboxing exists.** Landlock / Bubblewrap / macOS sandbox layers
  (`docs/SANDBOX.md`) already wrap agent command execution; the plugin host
  subprocess reuses them.
- **TUI renders no plugins** but its event match is deliberately exhaustive,
  so new plugin events force a compile-time decision there.
- `html_template` is carried end-to-end and rendered by nothing. It dies in
  this redesign (superseded by the declarative UI vocabulary).

## 2. Goals and non-goals

Goals:

1. Plugins are Rust `cdylib`s exposing a versioned, stable interface.
2. Plugins register **tools** that appear in the agent's toolset like built-in
   tools (schema advertised to the model, permissions, approval flow,
   streaming output).
3. Plugins register **UI** in defined slots of the clients.
4. **Load and unload at runtime** — no gateway restart to install, update,
   enable, disable, or remove a plugin.
5. Management UI (desktop first, TUI functional), an update flow, and a
   registry with signatures.

Non-goals (this iteration):

- A WASM runtime. Native dylibs are the decision; the API crate is designed so
  a WASM host could be added later as a third execution tier.
- Plugin-to-plugin dependencies.
- Sandboxing *in-process* plugins. The trusted tier is exactly as privileged
  as the gateway itself; that is what "trusted" means, and the UI must say so.

## 3. The ABI problem and the API crate

Rust has no stable ABI, so `rustc` is free to change layout between versions —
a dylib built by a different compiler than the host is undefined behaviour
waiting to happen. The industry answers are: (a) commit to `extern "C"` +
`#[repr(C)]` everywhere, hand-rolled vtables; (b) use `abi_stable`, which
automates exactly that and adds load-time layout checking; (c) `stabby`, the
younger alternative; (d) pin the exact toolchain and use plain Rust types.

**Decision: `abi_stable` for the gateway-side plugin interface.** It gives
`#[repr(C)]`-safe std types (`RString`, `RVec`, `ROption`, `RResult`), trait
objects across the boundary (`#[sabi_trait]`), and — critically — a layout
checksum verified at load time, turning silent UB into a clean "ABI mismatch"
error. Risk: `abi_stable` is maintained but not fast-moving; mitigation: it
never appears in plugin-author-facing signatures directly — everything routes
through our own `rustyclaw-plugin-api` types, so a future swap to `stabby` or
hand-rolled C ABI changes one crate, not every plugin.

The native *client UI* interface deliberately does **not** use `abi_stable`
(§7): Dioxus types cannot cross an FFI boundary, so UI dylibs use exact
toolchain pinning instead, with declarative fallback.

New crates:

| Crate | Kind | Purpose |
|---|---|---|
| `rustyclaw-plugin-api` | rlib | The stable contract: `PluginDecl`, tool/UI registration types, host capability handles. Both the gateway and every plugin depend on it. Semver = ABI version. |
| `rustyclaw-plugin-sdk` | rlib | Author ergonomics: `export_plugin!` macro, typed argument helpers, a test harness that loads a plugin in-process and drives its tools. Re-exports the api crate. |
| `rustyclaw-plugin-host` | bin | The subprocess that dlopens sandboxed-tier plugins and proxies calls over IPC. |
| `plugins/examples/*` | cdylib | At least two reference plugins (a tools-only one, a tools+UI one) built in-tree and used by integration tests. |

### The plugin declaration

A plugin exports one symbol. Sketch (final types to be settled in Phase 1):

```rust
// in the plugin crate (crate-type = ["cdylib"])
use rustyclaw_plugin_sdk::prelude::*;

export_plugin! {
    name: "github-dash",
    // Compiled-in constants the loader checks before calling anything:
    // api semver, rustc version string, target triple, layout checksum.
    init: |host: HostHandle| -> Result<PluginInstance, InitError> { ... }
}

impl Plugin for GithubDash {
    fn tools(&self) -> Vec<ToolSpec>;          // name, description, params, permission default
    fn invoke(&self, tool: &str, args: Json, ctx: CallCtx) -> ToolOutcome;
    fn ui(&self) -> Vec<UiContribution>;       // declarative panels/slots (§6)
    fn on_event(&self, event: HostEvent);      // config change, ui action, shutdown
    fn quiesce(&self);                         // finish/abort in-flight work before unload
}
```

`invoke` is **synchronous** at the ABI boundary, matching the existing
`SyncExecuteFn` architecture — the gateway already wraps sync tools onto a
blocking pool. Long-running tools stream via a host-provided `OutputSink`
callback (FFI-safe function object), mirroring `execute_tool_streaming`.
Async-over-FFI is a tarpit we deliberately stay out of; a plugin that wants a
runtime spawns its own.

`HostHandle` is the capability surface, and the only way a plugin reaches
RustyClaw facilities:

- `state()` — read/write/patch its persisted state (same TOML-on-disk store
  as today, same JSON-null caveats).
- `secrets(name)` — resolve a vault secret **the user has linked to this
  plugin** (reusing the skill-secret linking model). Never raw vault access.
- `emit_ui(patch)` — push a UI state update to clients.
- `log(level, msg)` — into the gateway's tracing, tagged with the plugin name.
- `http()` — an HTTP client that (for the sandboxed tier) enforces the
  manifest's declared network allowlist even if the plugin links its own
  networking, because the subprocess sandbox blocks other egress paths.

## 4. Execution tiers (hybrid model)

Every installed plugin has a **trust tier**, persisted per-plugin, changeable
only by the user (never by the agent, never by the manifest):

| Tier | Runs | Unload | Crash blast radius | Who gets it |
|---|---|---|---|---|
| `trusted` | In the gateway process, `libloading` | Quiesce + drop registrations + **leak the library** (never `dlclose`) | The gateway | Bundled plugins; plugins the user explicitly promotes |
| `sandboxed` | Per-plugin `rustyclaw-plugin-host` subprocess | Graceful shutdown message, then SIGKILL | That plugin only | Everything installed from the registry (default) |

Why never `dlclose` in-process: unloading a Rust dylib with TLS destructors,
`static`s, or spawned threads is a well-known source of use-after-free. The
trusted tier's "unload" quiesces the plugin, removes every registration, and
keeps the `Library` handle alive in a graveyard list for the life of the
process. Reload loads a *copy* of the file (into a per-version temp path — on
Windows the loaded DLL is locked anyway, so copy-before-load is mandatory
there) and registers the new instance. Leaked memory per reload is the
accepted cost; the manager UI shows a "restart gateway to fully reclaim"
hint after N reloads.

The host subprocess:

- Speaks length-prefixed bincode over stdin/stdout (same codec family as the
  wire protocol; *not* the SSH protocol itself — this is a private local IPC).
- Applies the existing sandbox stack (Landlock/Bubblewrap/macOS sandbox) with
  a profile computed from the manifest's declared capabilities: filesystem
  scope (workspace subtree by default), network on/off + host allowlist,
  no access to the settings dir or vault files, ever.
- Restart policy on crash: exponential backoff, 3 strikes → plugin marked
  `failed`, surfaced in the manager UI; in-flight tool calls return a tool
  error to the model (the agent can react) rather than wedging the turn.
- One host per plugin, not one shared host: isolation between plugins is the
  point, and the process count is small.

Both tiers implement the same internal `PluginInstance` trait inside the
gateway; the dispatcher cannot tell them apart. That keeps the hybrid model
from forking every code path.

## 5. Dynamic tools

The largest refactor, and it pays off even before dylibs exist.

### 5.1 Tool catalog

Replace `all_tools() -> Vec<&'static ToolDef>` with a `ToolCatalog`:

```rust
pub struct ToolCatalog { /* RwLock<...> */ }
impl ToolCatalog {
    pub fn snapshot(&self) -> Arc<ToolSet>;       // immutable, cheap to clone
    pub fn register(&self, source: ToolSource, tools: Vec<Arc<DynToolDef>>);
    pub fn unregister(&self, source: ToolSource); // source = Builtin | Plugin(name)
    pub fn generation(&self) -> u64;              // bumped on every change
}
```

- `DynToolDef` carries owned strings and an `enum Exec { Sync(fn), Native(Arc<dyn ..>), Remote(HostChannel) }`
  so built-ins keep their zero-cost path and plugin tools carry state.
- Built-ins register once at startup from the existing statics; the
  compile-time `#[cfg(feature)]` gating is unchanged.
- The three name-keyed side tables fold into `DynToolDef` fields
  (`params`, `summary`, `category`) so a plugin tool is a first-class citizen
  of the tool-config panel; `tool_toggle`'s "unknown name" rejection checks
  the catalog instead of `all_tools()`.
- `genai_backend.rs` takes a catalog snapshot **per model round**, not per
  process: a plugin loaded mid-conversation appears on the model's next turn,
  an unloaded one disappears. The `generation()` counter lets sessions skip
  schema rebuilds when nothing changed.

### 5.2 Naming, collisions, permissions

- Plugin tool names are namespaced by enforced prefix: a plugin `foo` may only
  register tools matching `foo_*` (provider tool-name charsets are
  `[a-zA-Z0-9_-]`, so no `:` or `.`). The catalog rejects violations and any
  collision with a built-in.
- Plugin tools default to permission **Ask** (approval required per call);
  the user can promote to Allow per tool in the existing tool-config panel.
  A *denied* tool is additionally filtered out of the advertised schema for
  plugin tools — no reason to spend prompt tokens on a tool the model cannot
  call. (Built-ins keep today's advertise-but-refuse behaviour for now; the
  discrepancy is noted in the panel.)
- The system prompt's plugin context (`prompt_context()`) is rebuilt from the
  catalog, and plugin-supplied instruction text is wrapped in the same
  untrusted-content framing used for skill instructions — a plugin manifest
  must not be able to inject "ignore your other tools".

## 6. Declarative UI (the baseline every plugin has)

The current dock — state JSON rendered as key-value pairs — becomes a real
widget vocabulary. This is the only UI path that works on the TUI, over SSH,
and for sandboxed plugins, so it is mandatory: **a plugin's UI must be
functional with declarative rendering alone**; native artifacts (§7) may only
enhance it.

- `UiContribution { slot: UiSlot, panel: PanelSpec }` where
  `UiSlot ∈ { Dock, Sidebar, Composer, Settings, StatusBar }` (desktop maps
  all five; TUI maps Dock and Settings, ignores the rest by explicit match).
- `PanelSpec` is a small layout tree: `Column/Row/Grid`, `Text/Badge/Progress`,
  `Table`, `Sparkline/Chart` (mapped to a TUI-safe subset), `Button`,
  `Input/Select/Toggle`, `Form`. Nodes bind to paths in the plugin's state
  JSON; the wire carries state *patches*, not re-renders.
- Interactions (`Button` press, `Form` submit) become a **new wire frame**
  `PluginUiEvent { plugin, element_id, payload }` → routed to the plugin's
  `on_event`. This replaces today's route-through-the-LLM action buttons; the
  "ask the agent to do it" button remains available as an explicit
  `Button { action: AgentPrompt(..) }` variant, because it is genuinely useful
  — it just stops being the *only* mechanism.
- The existing TOML-only plugins keep working: a manifest with state schema +
  actions and no dylib is a "declarative plugin", auto-rendered exactly as
  today (modulo the new renderer), zero migration required.

## 7. Native client UI (the enhancement tier)

Per the owner decision, plugins may ship native desktop UI. Honest constraint:
Dioxus `Element`s cannot cross an `abi_stable` boundary — they are full of
non-`repr(C)` generics — so native UI dylibs use **exact-match pinning**
instead of a stable ABI:

- A UI artifact is valid only for an exact **UI-ABI tuple**:
  `(target triple, rustc version, dioxus version, rustyclaw-ui-api version)`.
  The tuple is compiled into the artifact and checked byte-for-byte at load;
  any mismatch → the plugin silently falls back to its declarative panel.
  RustyClaw releases publish their tuple so plugin authors can build against
  it (`rustyclaw plugin build --ui` fetches the right toolchain via rustup).
- The artifact exports `fn mount(ctx: UiCtx) -> Element` plus the tuple
  consts; it is loaded with `libloading` into the desktop process and — same
  rule as the gateway — never `dlclose`d, only quiesced and leaked on
  unload/update.
- The desktop fetches UI artifacts itself (the gateway may be on another
  machine): on `PluginsUpdate` it sees which plugins advertise a UI artifact
  for its tuple, downloads from the registry (or from the gateway acting as a
  cache) into a per-tuple local cache, verifies the signature (§9), and loads.
- Loading native code into the *client* is a separate consent: the desktop
  asks once per plugin ("load native UI from <publisher>?"), and a
  settings toggle disables native UI globally (declarative fallback
  everywhere). The TUI never loads native UI.

This tier is last in the build order (Phase 5) and the plan treats it as
strictly optional polish: everything must already work without it.

## 8. Runtime lifecycle and the wire protocol

Plugin instance state machine (gateway-side):

```
Discovered → Loaded → Enabled ⇄ Disabled
                │        │
                │        └→ Quiescing → Unloaded
                └→ Failed (load error / ABI mismatch / 3 crashes)
```

- **Enabled** = tools in the catalog, UI contributions pushed.
- **Disable** = unregister tools + retract UI, keep the instance warm.
- **Unload** = disable, `quiesce()` with a deadline (default 10 s), then
  drop/kill per tier.
- **Update** = download + verify new version → load it (new instance) →
  atomically swap registrations → unload old. A failed new-version load
  leaves the old one running.
- In-flight tool calls during disable/unload run to completion within the
  quiesce deadline, then are aborted with a tool error.

New wire frames (new discriminants; existing plugin frames stay for
back-compat during the transition, `WIRE_PROTOCOL_VERSION` bumps once at the
start of the work):

- Client→gateway: `PluginInstall{source}`, `PluginUninstall{name}`,
  `PluginSetEnabled{name, enabled}`, `PluginSetTier{name, tier}`,
  `PluginUpdate{name, version}`, `PluginUiEvent{...}`,
  `PluginRegistrySearch{query}`, `PluginDetail{name}`.
- Gateway→client: a richer `PluginsUpdate2` (list with tier/status/version/
  pending-update/capabilities), `PluginUiPatch{plugin, patch}`,
  `PluginInstallProgress{...}`.
- All mutating commands are **user-only surfaces** (manager UI / CLI); the
  agent-facing tools do not include install/tier operations. The agent may
  *search* the registry and *suggest* installation; installing native code is
  a human decision made in the client, in line with the guard-override
  philosophy already in the codebase.

## 9. The registry

A separate, purpose-built service (working name: **ClawForge**; final name
TBD) because native artifacts change the threat model completely: a Markdown
skill misleads a model; a `.so` owns the machine.

Contract-first design — the API is a versioned spec in-repo so the server can
be implemented (and self-hosted) independently:

- `GET /api/v1/plugins?q=` — search (name, description, publisher, downloads).
- `GET /api/v1/plugins/{name}` — metadata: versions, per-version artifact
  matrix (target triple × kind[gateway|ui-tuple]), capability declarations,
  publisher identity, signature info.
- `GET /api/v1/plugins/{name}/{version}/artifact?target=&kind=` — the binary.
- `POST /api/v1/publish` — device-code auth (same UX as ClawHub's CLI flow),
  multi-artifact upload with a signed manifest.
- Yank/deprecate endpoints; no deletion of released bytes.

Security model (mandatory from day one, not retrofitted):

- **Publisher signing.** Publishers hold an ed25519 key (`rustyclaw plugin
  keygen`); every artifact manifest (name, version, capability declarations,
  per-artifact SHA-256 set) is publisher-signed. The registry countersigns on
  publish (timestamp + identity binding). Install verifies both signatures
  and every artifact hash before any byte lands in the plugins dir; the
  first install pins the publisher key (TOFU), and a key change on update is
  a hard, loud prompt.
- **Capability review surface.** The install UI shows the manifest's declared
  capabilities (network hosts, filesystem scope, secrets requested) *before*
  confirmation — the Android-permissions moment — and the default tier is
  always `sandboxed`.
- Self-hosting: the read side is deliberately static-file-shaped (JSON index +
  content-addressed artifacts), so an org can serve it from any file host;
  `registry_url` is configurable exactly like ClawHub's is today.

Build/publish tooling: `rustyclaw plugin new` (cargo-generate template),
`build` (per-target, cross-aware), `test` (SDK harness), `publish`. CI recipe
in the template for building the target matrix on GitHub Actions.

## 10. Management UI

Desktop — a Plugins manager (replaces the thin dock header, lives beside
Settings):

- **Installed** tab: per plugin — status (with crash/failed badges), version,
  tier (with promote/demote, promote gated behind a scary-and-clear dialog),
  capabilities granted, linked secrets, enable/disable, unload/reload,
  uninstall, update-available indicator + one-click update.
- **Browse** tab: registry search, detail view (capabilities, publisher,
  signature status, targets), install.
- **Updates** tab: all pending updates, changelogs, update-all. Auto-update is
  opt-in and only ever within the tier the user set.
- Per-plugin log tail (host-subprocess stderr / tagged gateway tracing).

TUI: installed list with enable/disable/uninstall and update indicator —
functional parity for management, no registry browsing in v1.

CLI: `rustyclaw plugin list|install|uninstall|enable|disable|update|search|info`,
flag-compatible with the skills commands where the concepts overlap.

## 11. Security summary

Consolidated, because it cuts across everything above:

1. Native code in the trusted tier **is** the gateway; the UI says so
   whenever a user promotes a plugin.
2. Sandboxed tier: subprocess + Landlock/Bubblewrap/macOS profile from
   declared capabilities; no vault/settings access; network allowlisted.
3. Registry: dual signatures + hash pinning + TOFU publisher keys + capability
   display at install; yank support.
4. Secrets: only via host capability, only user-linked, never raw vault.
5. Agent boundaries: the agent cannot install, promote, or grant; plugin
   instruction text is framed as untrusted in the system prompt.
6. Client-side native UI: separate consent, per-tuple verification, global
   off-switch, TUI never loads code.

## 12. Phasing

Each phase lands independently and is useful without the ones after it.

- **Phase 0 — dynamic foundations (no dylibs yet).**
  `ToolCatalog` refactor (5.1–5.2) with built-ins registered at startup;
  per-round schema snapshots in `genai_backend`; wire-protocol additions
  (`PluginUiEvent`, `PluginsUpdate2`, enable/disable); real action execution
  for declarative plugins replacing the route-through-LLM hack; TUI dock
  (declarative renderer, subset). *Risk retired: the static-registry
  assumption, which touches the most existing code.*
- **Phase 1 — the ABI and in-process loading.**
  `rustyclaw-plugin-api` + `-sdk` + `export_plugin!`; loader with layout/
  version checks; trusted-tier lifecycle (load/enable/disable/quiesce/leak);
  example plugins + SDK test harness; CLI `plugin load/unload` against a
  local path. *Risk retired: abi_stable viability.*
- **Phase 2 — sandboxed tier.**
  `rustyclaw-plugin-host`, IPC, sandbox profiles from capability manifests,
  crash/restart policy, tier plumbing + `PluginSetTier`. *Risk retired:
  process supervision and capability enforcement.*
- **Phase 3 — declarative UI vocabulary + manager UI.**
  `PanelSpec` widget tree + state patching; desktop manager (Installed tab,
  local install flows); TUI management list.
- **Phase 4 — registry.**
  API spec; client (search/install/verify/update); signing toolchain;
  publish flow; Browse/Updates tabs; a minimal reference server
  implementation (static-index shape) in a sibling repo.
- **Phase 5 — native client UI.**
  UI-ABI tuple pinning, desktop artifact cache + consent + loading,
  fallback behaviour, `plugin build --ui` toolchain automation.

Suggested checkpoint after Phase 1: publish the API crate as 0.x and build one
real (dogfood) plugin before freezing anything — the SDK's ergonomics will
only be believable with a real consumer.

## 13. Open questions

1. Registry name and hosting (domain, who operates it, funding).
2. Whether Phase 0's advertised-schema filtering for denied tools should
   extend to built-ins (prompt-size win vs. behaviour change).
3. Minimum supported client set for declarative UI v1 — is web
   (`rustyclaw-web`) in scope?
4. Whether trusted-tier promotion requires the vault passphrase (stronger
   than a dialog; annoying; probably right).
5. Bundling: which first-party capabilities migrate out of the monolith into
   bundled plugins once the platform exists (messengers? engines? freenet
   tools?) — the original modularization goal. Candidate for a follow-up
   plan once Phase 2 is real.
