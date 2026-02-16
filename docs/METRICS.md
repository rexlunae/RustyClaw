# Metrics

RustyClaw exposes Prometheus metrics when enabled.

Config:

```toml
[metrics]
enabled = true
listen = "127.0.0.1:9090"
```

Endpoints:

- `GET /metrics`
- `GET /health`

Metric families:

- `rustyclaw_gateway_connections`
- `rustyclaw_auth_attempts_total{result}`
- `rustyclaw_request_duration_seconds{request_type}`
- `rustyclaw_tool_calls_total{tool_name,result}`
- `rustyclaw_provider_requests_total{provider,result}`
- `rustyclaw_tokens_total{provider,type}`
- `rustyclaw_prompt_injection_detected_total{action}`
- `rustyclaw_ssrf_blocked_total{reason}`

Implementation: `src/metrics.rs`
