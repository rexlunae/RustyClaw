/// Thread/task info for TUI display (unified).
///
/// Reuses the server-frame `ThreadInfoDto` from `rustyclaw-core` since the
/// TUI directly consumes gateway server frames.
pub type ThreadInfo = rustyclaw_core::gateway::protocol::ThreadInfoDto;
