# Gateway Protocol

RustyClaw gateway provides a WebSocket interface for TUI and external clients.

Start gateway:

```bash
rustyclaw gateway start
```

Default endpoint:

- `ws://127.0.0.1:8080`
- `wss://...` when TLS is enabled

Message flow (high-level):

1. Client connects and receives `hello`
2. Client sends chat payload (`type = "chat"`)
3. Gateway streams model output (`response_chunk`, `response_done`)
4. Tool calls are emitted as `tool_call` / `tool_result`

References:

- Gateway main loop: `src/gateway/mod.rs`
- Protocol tests: `tests/gateway_protocol.rs`
- TLS config: `docs/SECURITY.md`, `docs/SANDBOX.md`
