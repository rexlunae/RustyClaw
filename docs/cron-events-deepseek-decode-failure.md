# Incident: Scheduled (cron) turns fail against DeepSeek while interactive turns work

Date: 2026-08-10 · Job: `morning-ai-bubble-briefing` · Affects: all `Payload::AgentTurn` cron wakes

## Summary

The cron scheduler ("the events system") fired its agent-turn job on the morning of
2026-08-10, but the model call failed with a **reqwest decode error**, and the run
**hung for 5.6 hours** before erroring out. Interactive turns against the same
DeepSeek model worked throughout. The failure is therefore specific to the
scheduled-turn code path, not the DeepSeek API.

## Evidence

Runs log (`~/.rustyclaw/cron/runs/morning-ai-bubble-briefing.jsonl`), two distinct failures:

1. **400 Bad Request — model name sent with provider prefix**

   ```
   Web call failed for model 'deepseek/deepseek-v4-flash (adapter: OpenAI)'.
   Cause: Request failed with status code '400 Bad Request'. Response body:
   {"error":{"message":"The supported API model names are deepseek-v4-pro or
   deepseek-v4-flash, but you passed deepseek/deepseek-v4-flash.", ...}}
   ```

   The cron path copies `model_ctx.model` verbatim into the provider request
   (`cron_runtime.rs` → `run_agent_turn` → `ProviderRequest { model: ... }`).
   genai's OpenAI adapter sends the model name on the wire as-is (only a `::`
   namespace prefix is stripped; a `provider/` slash prefix is not), so a model
   configured as `deepseek/deepseek-v4-flash` reaches the API and is rejected.
   Later runs succeeded, so this was corrected in config — but nothing in the
   cron path normalises or validates the model name.

2. **Reqwest decode error + 5.6h hang**

   ```
   Web call failed for model 'deepseek-v4-flash (adapter: OpenAI)'.
   Cause: Reqwest error: error decoding response body for url
   (https://api.deepseek.com/v1/chat/completions)
   ```

   run `run-19feb0b158b`: started 09:40:00 UTC, finished 15:17:21 UTC, status
   `error`. The event fired, the model call failed, and the run stayed
   `running` for 5.6 hours before the error surfaced.

## Root-cause analysis

### 1. Cron turns have no model-call timeout (the 5.6h hang)

Every other runner bounds the model call; cron does not:

| Path | Call site | Timeout |
|---|---|---|
| Interactive | `dispatch.rs` `await_model_with_cancel(..., 180s)` | ✅ 180 s |
| Spawned run | `spawn_runner.rs` `tokio::time::timeout(MODEL_CALL_TIMEOUT, ...)` | ✅ 300 s |
| Subagent | `subagent_runner.rs` `tokio::time::timeout(MODEL_CALL_TIMEOUT, ...)` | ✅ 300 s |
| **Cron** | `cron_runtime.rs` `call_with_tools(&http, &resolved, None).await?` | ❌ **none** |

The reqwest client carries a 180 s *read* timeout, but that only fires on
silence between bytes; a connection that trickles (or a provider edge that
stalls before the body completes) is not bounded by it. The run stayed
"running" for 5.6 h — the scheduler records `Running` at fire and only writes
the terminal state when the turn returns.

### 2. Cron uses the non-streaming `exec_chat` path (the decode error)

- Interactive turns: `call_with_tools(..., Some(writer))` → `exec_chat_stream`
  (SSE). This is the path that works against DeepSeek.
- Cron/trigger/spawn/subagent turns: `call_with_tools(..., None)` →
  `exec_chat` (batch JSON).

The non-streaming path ends in `genai::webc::WebResponse::from_reqwest_response`
→ `res.text()`. The reported error — reqwest `Kind::Decode`, displayed as
"error decoding response body for url" — is **not** a UTF-8 decode failure. In
reqwest 0.13.4, `Kind::Decode` is the wrapper for *any* body-stream read
failure (`BodyExt::collect` error, `response.rs:435`): a truncated gzip stream,
a connection dropped mid-body, or a content-length mismatch. A connection that
dies mid-response (DeepSeek's edge, a NAT rebind, a proxy) surfaces here.

**Verified non-cause:** gzip decompression was suspected first, but reqwest
0.13.4 enables gzip **by default** when the feature is compiled (`Accepts`
default has `gzip: true`), and the feature is unified across the workspace via
genai's dependency. `res.text()` failing to decode gzip bytes is therefore not
the mechanism — the body stream itself failed to be read.

**Why cron is the path that hangs:** the 180 s `read_timeout` on the shared
provider client is **per-chunk**, not a total deadline — it resets on every
byte of activity. A connection that trickles a byte every minute (or stalls
between chunks) never trips it. Interactive turns are additionally bounded by
`await_model_with_cancel(..., 180s)` around the whole call; cron turns have no
equivalent, so a trickling/stalled connection runs for hours until something
far below the application gives up — exactly the 5.6 h observed.

### 3. Error reporting hides the response body

`WebModelCall` wraps the reqwest error but does not include the raw response
body (`capture_raw_body` is not enabled on the cron options), so the actual
bytes that failed to decode are not visible in the run log. Diagnosis required
code reading plus the runs log.

## Recommendations

1. **Give cron turns a total model-call deadline** matching the other runners
   (e.g. 300 s via `tokio::time::timeout`), and ideally a total-run deadline so
   a wedged turn cannot pin the scheduler's sequential `for job in due { ... }`
   loop for hours. The per-chunk `read_timeout` is not a substitute.
2. **Normalise/validate the model name** in the cron path (strip a
   `provider/` prefix, or reject it), so a config like
   `deepseek/deepseek-v4-flash` cannot be sent verbatim to the API.
3. **Surface the raw body on decode errors** (enable `capture_raw_body` or
   include the failing bytes in `WebModelCall`-adjacent error reporting) so the
   next occurrence is diagnosable from the runs log alone.

## Files involved

- `crates/rustyclaw-gateway/src/cron_runtime.rs` — scheduled turn, no total
  deadline, model name passed verbatim
- `crates/rustyclaw-core/src/providers/genai_backend.rs` — non-streaming vs streaming dispatch
- `vendor/genai/src/webc/web_client.rs` — `res.text()` decode failure site
- `crates/rustyclaw-gateway/src/dispatch.rs` — interactive path (streaming, 180 s total timeout)
- reqwest 0.13.4 — `Kind::Decode` wraps body-stream read failures
  (`src/async_impl/response.rs:435`); `read_timeout` is per-chunk, not total
