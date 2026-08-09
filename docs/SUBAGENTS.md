# Focused Subagents

Subagents are narrowly-scoped agents that a main agent spins up for
well-defined jobs. Each subagent runs from a **profile**: a tight,
job-specific system prompt plus an explicit allowlist of the tools that job
needs — and nothing else. The point is focus: a subagent is not distracted
by the main agent's accumulated conversation, its memory files, or the full
tool registry.

## How a run works

1. The main agent calls `subagent_run(profile="…", task="…", context="…")`.
2. The gateway intercepts the call (subagents need model credentials, which
   only the gateway holds) and starts a fresh, headless tool loop:
   - **System prompt** = the profile's prompt + isolated workspace context
     (`SessionType::Isolated` — no `MEMORY.md`/`USER.md`) + a listing of
     the profile's exact toolset + autonomous-session rules.
   - **Conversation** = only the task and the context the parent fed it.
   - **Tools** = only the profile's allowlist. This is enforced twice:
     the model's requests only ever *present* the allowlisted tool schemas
     (`ProviderRequest::allowed_tools`), and the executor refuses any call
     outside the allowlist as defense in depth.
3. The loop runs until the subagent produces a final message (bounded by
   the profile's `max_rounds`, default 24, ceiling 60). The user's per-tool
   permission policy applies: since no user is present to approve, anything
   not set to `Allow` is refused, mirroring trigger-fired runs.
4. The final message is returned to the main agent as the `subagent_run`
   tool result. The run is recorded in the session manager, so
   `sessions_list` and `sessions_history` show it.

The run is synchronous from the parent's perspective — the parent's tool
call blocks until the subagent finishes.

## Background runs: `sessions_spawn` and `sessions_kill`

When the parent should *not* wait, `sessions_spawn` starts a full agent in
the background and returns a `sessionKey` immediately. The parent keeps
working and polls with `sessions_history` or `session_status`.

Because the turn that started it has long since returned, a background run
plays by stricter rules than a `subagent_run`:

- **Nothing else will stop it.** `sessions_kill(sessionKey=…)` — or
  `label=…` — is the only thing that ends a run before it finishes. It
  stops the whole subtree: a spawned run's own spawns are owned by an
  identity that disappears with it, so leaving them behind would make them
  unstoppable. The session history stays readable afterwards.
- **It cannot ask you anything.** No user is attached, so any tool your
  permission policy does not set to `Allow` is refused rather than left
  waiting for an approval that can never arrive. Do not delegate work that
  needs an approval or an answer.
- **It is bounded.** 25 tool rounds, 300s per model call, and an optional
  `runTimeoutSeconds`. Across the whole process,
  `tool_limits.max_background_sessions` (default 32) caps how many runs can
  be alive at once — a per-caller limit cannot bound depth, since each
  spawned run is a caller in its own right.

A run's history records which tools it called and whether they succeeded,
but never their output: session records are broadly readable, and tool
results and error messages can carry secrets.

Only a background run can be stopped this way. `sessions_kill` refuses a
main session or a synchronous `subagent_run`, rather than reporting
success for work it cannot actually interrupt.

`sessions_spawn` takes `task`, and optionally `label`, `agentId`, `model`,
and `runTimeoutSeconds`. (`thinking` and `cleanup` were previously listed
but never implemented; they have been removed.)

## Built-in profiles

| Profile | Job | Toolset |
|---|---|---|
| `code-writer` | Implement a precisely-described change, verify it builds | file read/write/edit/patch, search, execute_command, process, ast_grep |
| `code-reviewer` | Review code without modifying it | file read, search, execute_command (read-only checks), ast_grep |
| `bug-hunter` | Reproduce and root-cause a bug, without fixing it | file read, search, execute_command, process, ast_grep |
| `test-writer` | Write and run tests following project conventions | file read/write/edit/patch, search, execute_command, process, ast_grep |
| `researcher` | Research a question, return sourced findings | web_search, web_fetch, web_extract, file read, search |
| `doc-writer` | Write docs that match the code as it is | file read/write/edit, search |

`subagent_list` shows these plus any custom profiles.

## Custom profiles

The main agent (or you, by dropping a TOML file) can create profiles when a
recurring job doesn't fit a built-in:

```
subagent_create(
  name="License Checker",
  description="Audits dependency licenses",
  system_prompt="You audit dependency licenses. …",
  tools=["read_file", "search_files", "execute_command"],
  max_rounds=12,
)
```

Custom profiles are persisted as `<settings_dir>/subagents/<id>.toml` and
deleted with `subagent_delete(id="…")`. Built-in ids are reserved and
cannot be shadowed or deleted.

### Toolset policy

Profile toolsets are validated against the real tool registry, and some
tools can never appear in a subagent's toolset:

- **Interactive tools** (`ask_user`, `client_dom_query`) — no user is
  present in a subagent session.
- **Agent/session management** (`subagent_*`, `sessions_*`, `agents_*`,
  `swarm_*`, `triggers_*`) — subagents must not spawn or manage other
  agents (no recursion).
- **Gateway-manager families not wired into the headless loop**
  (`secrets_*`, `skill_*`, `mcp_*`, `task_*`, `model_*`, `service_*`,
  `plugin_*`).
- **Outward messaging and installation-level operations** (`message`,
  `tts`, `gateway`, `cron`, sysadmin tools, `secure_delete`, …) — route
  through the parent instead.
- **Shared-state mutation** (`save_memory`, `add_memory`, `todo`, thread
  metadata) — a subagent reports; the parent decides what to persist.

## Feeding context

A subagent cannot see the parent conversation. The `context` parameter is
how the main agent forwards what matters: file paths, prior findings,
constraints, error output. The built-in prompt guidance tells the main
agent to pass **all** relevant context explicitly, and tells the subagent
to report exactly what was missing if it couldn't proceed.

## Model selection

A run reuses the parent's provider credentials. The model can be overridden
per profile (`model` in the profile) or per run (`model` argument to
`subagent_run`), as long as the model belongs to the same provider —
useful for sending simple jobs to cheaper models.
