# Tools Reference

RustyClaw exposes a tool registry for agent execution.

Core categories include:

- File: `read_file`, `write_file`, `edit_file`, `list_directory`, `search_files`, `find_files`
- Runtime: `execute_command`, `process`, `apply_patch`
- Web: `web_fetch`, `web_search`
- Memory: `memory_search`, `memory_get`
- Scheduling: `cron`
- Sessions: `sessions_list`, `sessions_spawn`, `sessions_send`, `sessions_history`, `session_status`, `agents_list`
- Secrets: `secrets_list`, `secrets_get`, `secrets_store`
- System: `gateway`, `message`, `tts`
- Device/visual: `image`, `nodes`, `browser`, `canvas`

Implementation entry points:

- Tool registry: `src/tools/mod.rs`
- Tool schemas: `src/tools/params.rs`
- Tool runtime helpers: `src/tools/helpers.rs`
