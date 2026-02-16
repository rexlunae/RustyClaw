# RustyClaw Development Roadmap
## Issues #51-#94 Prioritization Analysis

**Generated**: 2026-02-16
**Analysis Basis**: Implementation complexity, ecosystem reference availability, effort estimation

---

## Executive Summary

This roadmap analyzes 44 feature requests (#51-#94) and prioritizes them across 4 tiers based on:
1. **Implementation Complexity**: Lines of code, dependencies, integration points, testing requirements
2. **Reference Availability**: Number of ecosystem implementations (OpenClaw, AutoGPT, PicoClaw, Moltis, MicroClaw, Carapace)
3. **Estimated Effort**: From issue descriptions (1-6 weeks)
4. **Strategic Value**: Security, reliability, user experience impact

**Key Insight**: Issues with 3+ ecosystem references have proven implementations to learn from, reducing risk and accelerating development.

---

## Priority Tier Distribution

| Tier | Count | Description |
|------|-------|-------------|
| **P0 - Quick Wins** | 10 issues | 1-2 weeks, high impact, low complexity |
| **P1 - High Value** | 12 issues | 2-3 weeks, critical features, medium complexity |
| **P2 - Medium Complexity** | 14 issues | 3-4 weeks, valuable enhancements, higher integration needs |
| **P3 - Advanced Features** | 8 issues | 4-6 weeks, complex systems, significant dependencies |

---

## P0: Quick Wins (1-2 weeks each)
*Maximize velocity with high-impact, low-complexity features*

### Security & Reliability Quick Wins

#### #52 - Unified Safety Layer Consolidation ⚡
**Effort**: 1-2 weeks | **Complexity**: LOW | **References**: 0 (IronClaw inspired)
**Why P0**: Critical security consolidation, existing code to refactor, clear architecture
**LOC Estimate**: 400-600 lines
**Dependencies**: None (refactoring existing code)
**Priority Justification**: Security is foundational; consolidation simplifies maintenance

**Implementation Path**:
1. Create `src/safety/mod.rs` with 4-component architecture
2. Migrate existing SSRF detector → Validator
3. Migrate existing prompt guard → Sanitizer
4. Add LeakDetector for credential patterns
5. Add Policy engine (warn/block/sanitize modes)

---

#### #86 - Secure Credential Memory (zeroize/secrecy) ⚡
**Effort**: 1 week | **Complexity**: LOW | **References**: 2 (Moltis, Carapace)
**Why P0**: Quick security win, well-understood pattern, minimal code changes
**LOC Estimate**: 200-300 lines
**Dependencies**: `secrecy = "0.8"`, `zeroize = "1.7"`
**Priority Justification**: Addresses security issue #9 (H2), simple wrapper pattern

**Implementation Path**:
1. Add `secrecy` and `zeroize` to Cargo.toml
2. Wrap vault password in `Secret<String>`
3. Wrap decrypted API keys in `Secret<String>`
4. Update `Debug` impls to use `[REDACTED]`
5. Add unit tests verifying no secret leakage

---

#### #81 - Structured Retry/Backoff Engine ⚡
**Effort**: 1-2 weeks | **Complexity**: LOW-MEDIUM | **References**: 3 (OpenClaw, Moltis, MicroClaw)
**Why P0**: Improves reliability across all external APIs, reusable component
**LOC Estimate**: 300-400 lines
**Dependencies**: None (use existing `tokio`)
**Priority Justification**: Foundation for failover (#51) and rate limiting (#69)

**Implementation Path**:
1. Create `src/retry/policy.rs` with `RetryPolicy` struct
2. Implement exponential backoff with jitter
3. Add `Retry-After` header parsing
4. Create error classification enum
5. Implement `retry_with_backoff()` async helper
6. Add metrics tracking (retry count, delays)

---

#### #70 - CSRF Protection for Gateway ⚡
**Effort**: 1 week | **Complexity**: LOW | **References**: 2 (Carapace, Moltis)
**Why P0**: Security quick win, standard pattern, minimal code
**LOC Estimate**: 150-200 lines
**Dependencies**: None (use existing crypto primitives)
**Priority Justification**: Essential for web gateway security

**Implementation Path**:
1. Add CSRF token generation (32-byte random)
2. Add in-memory token store with 1-hour TTL
3. Create middleware for POST /control/* validation
4. Add GET /csrf endpoint for token retrieval
5. Add tests for token validation and expiry

---

#### #83 - Config Validation with Unknown Field Detection ⚡
**Effort**: 1-2 weeks | **Complexity**: MEDIUM | **References**: 3 (Moltis, OpenClaw, PicoClaw)
**Why P0**: Prevents silent failures, excellent UX, well-documented pattern
**LOC Estimate**: 400-500 lines
**Dependencies**: Use existing `serde` deserialize hooks
**Priority Justification**: Catches user errors before they cause issues

**Implementation Path**:
1. Create diagnostic categories (Error, Warning, Info)
2. Add unknown field detection via `serde(deny_unknown_fields)`
3. Implement Levenshtein distance for typo suggestions
4. Add security warnings (unsafe paths, weak settings)
5. Create `rustyclaw config validate` command

---

#### #84 - Workspace Personality Files (SOUL.md, IDENTITY.md, USER.md) ⚡
**Effort**: 1 week | **Complexity**: LOW | **References**: 3 (PicoClaw, MicroClaw, Moltis)
**Why P0**: Simple file loading, high user value, clear pattern
**LOC Estimate**: 200-250 lines
**Dependencies**: None (use existing file readers)
**Priority Justification**: Enables easy customization without code changes

**Implementation Path**:
1. Add `load_workspace_files()` function
2. Check for SOUL.md, IDENTITY.md, USER.md, AGENTS.md, HEARTBEAT.md, TOOLS.md
3. Inject contents into system prompt dynamically
4. Add template files to `rustyclaw init`
5. Document personality file format

---

#### #85 - DuckDuckGo Fallback for web_search ⚡
**Effort**: 1 week | **Complexity**: LOW | **References**: 3 (PicoClaw, Moltis, OpenClaw)
**Why P0**: Zero-config search, excellent UX, simple HTML parsing
**LOC Estimate**: 250-300 lines
**Dependencies**: None (use existing reqwest)
**Priority Justification**: Removes API key barrier for basic search

**Implementation Path**:
1. Create `SearchProvider` trait
2. Keep existing Brave implementation
3. Add DuckDuckGo HTML parser (links + snippets)
4. Add automatic fallback on Brave API errors
5. Add config option for preferred provider

---

#### #63 - Heartbeat System for Proactive Monitoring ⚡
**Effort**: 1 week | **Complexity**: LOW | **References**: 0 (IronClaw inspired)
**Why P0**: Simple cron-like feature, low complexity, high value for monitoring
**LOC Estimate**: 200-250 lines
**Dependencies**: Use existing `tokio` interval
**Priority Justification**: Enables proactive monitoring without complexity

**Implementation Path**:
1. Read HEARTBEAT.md checklist from workspace
2. Create background tokio task with configurable interval
3. Execute agent turn with checklist as prompt
4. Add enable/disable toggle in config
5. Add tests for interval triggering

---

#### #73 - Gateway Service Lifecycle Management ⚡
**Effort**: 1-2 weeks | **Complexity**: MEDIUM | **References**: 3 (MicroClaw, Moltis, OpenClaw)
**Why P0**: Essential for production deployment, well-documented patterns
**LOC Estimate**: 500-600 lines
**Dependencies**: None (system tools)
**Priority Justification**: Required for persistent gateway operation

**Implementation Path**:
1. Add `rustyclaw gateway install` command
2. Generate systemd unit file (Linux)
3. Generate launchd plist (macOS)
4. Implement start/stop/status/logs commands
5. Add automatic log rotation (hourly, 30-day retention)

---

#### #51 - Multi-provider LLM Failover
**Effort**: 1-2 weeks | **Complexity**: MEDIUM | **References**: 0 (IronClaw inspired)
**Why P0**: Critical reliability feature, builds on #81 retry engine
**LOC Estimate**: 400-500 lines
**Dependencies**: Requires #81 (retry engine)
**Priority Justification**: Eliminates single point of failure

**Implementation Path**:
1. Create `FailoverProvider` wrapping multiple providers
2. Implement error classification (retryable vs not)
3. Add round-robin or priority-based selection
4. Track cost per provider used
5. Add configuration via `[[llm.failover]]` array

---

## P1: High Value Features (2-3 weeks each)
*Critical features with medium complexity*

### Enhanced LLM & Memory

#### #53 - Context Compaction for Long Conversations
**Effort**: 1-2 weeks | **Complexity**: MEDIUM | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 600-800 lines
**Dependencies**: Uses existing LLM provider
**Priority Justification**: Enables indefinitely long conversations

**Strategies**:
- Summarize: LLM generates executive summary of old messages
- SlidingWindow: Keep first N + last N messages
- Importance: Score messages by semantic relevance
- Hybrid: Combine strategies

---

#### #76 - Structured Memory with Auto-Reflector
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 3 (MicroClaw, Moltis, AutoGPT)
**LOC Estimate**: 800-1000 lines
**Dependencies**: SQLite (already present), optional embeddings
**Priority Justification**: Persistent agent memory is highly differentiating

**Components**:
1. File memory: AGENTS.md for manual facts
2. Structured memory: SQLite with auto-extracted facts
3. Background reflector: Periodically extracts durable facts
4. Quality gates: Confidence scoring, deduplication
5. Optional semantic retrieval (vector search)

---

#### #56 - Hybrid Search (BM25 + Vector) with RRF
**Effort**: 2-3 weeks | **Complexity**: HIGH | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 700-900 lines
**Dependencies**: `tantivy` (BM25), embeddings from #54
**Priority Justification**: Significantly better context retrieval

**Implementation**:
1. BM25 full-text index with `tantivy`
2. Vector similarity search with HNSW
3. Reciprocal Rank Fusion (RRF) to merge results
4. Configurable weight (0.5 = balanced)

---

### Agent Capabilities

#### #66 - Sub-agent / spawn_agent Tool
**Effort**: 2-3 weeks | **Complexity**: MEDIUM-HIGH | **References**: 3 (MicroClaw, Moltis, PicoClaw)
**LOC Estimate**: 600-800 lines
**Dependencies**: None (uses existing agent engine)
**Priority Justification**: Enables parallel task delegation

**Design**:
- Restricted tool registry (no recursive spawning, no cross-chat messaging)
- Configurable nesting depth limit (default 5)
- Inherit sandbox/security policies from parent
- Timeout enforcement per sub-agent
- Async execution (PicoClaw model)

---

#### #78 - Extensible Lifecycle Hook System
**Effort**: 2-3 weeks | **Complexity**: MEDIUM-HIGH | **References**: 2 (Moltis, OpenClaw)
**LOC Estimate**: 700-900 lines
**Dependencies**: None
**Priority Justification**: Enables community plugins without code changes

**Hook Types** (13+):
- BeforeToolCall, AfterToolCall
- MessageReceived, MessageSending, MessageSent
- BeforeCompaction, AfterCompaction
- ToolResultPersist
- SessionStart, SessionEnd
- GatewayStart, GatewayStop
- Command

**Features**:
- Shell hook protocol (stdin/stdout JSON)
- Priority ordering for multiple hooks
- Circuit breaker protection
- Async/sync execution modes

---

### Security Enhancements

#### #67 - LLM-based Prompt Injection Classifier
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 2 (Carapace, OpenClaw)
**LOC Estimate**: 500-600 lines
**Dependencies**: Uses existing LLM provider
**Priority Justification**: More robust than pattern matching

**Features**:
- Two-pass detection: pattern filter (fast) → LLM classifier (thorough)
- Configurable modes: off/warn/block
- Configurable threshold (0.0-1.0, default 0.8)
- Circuit breaker: disable after N failures
- Minimal latency impact (concurrent with response)

---

#### #69 - Per-IP Rate Limiting with Token Bucket
**Effort**: 1-2 weeks | **Complexity**: MEDIUM | **References**: 3 (Carapace, Moltis, MicroClaw)
**LOC Estimate**: 400-500 lines
**Dependencies**: None
**Priority Justification**: Essential DoS protection

**Implementation**:
- Token bucket algorithm per client IP
- Configurable: capacity, refill rate, burst allowance
- HTTP 429 + Retry-After header
- Per-route limits (stricter for auth endpoints)

---

#### #71 - PII Redaction and Output Sanitizer
**Effort**: 2-3 weeks | **Complexity**: MEDIUM-HIGH | **References**: 2 (Carapace, Moltis)
**LOC Estimate**: 600-700 lines
**Dependencies**: Regex patterns
**Priority Justification**: Privacy protection for agent outputs

**Detection Patterns**:
- Email addresses
- Phone numbers
- Social Security Numbers
- Credit card numbers
- API keys, tokens, passwords
- XSS prevention (dangerous HTML)
- Data URI stripping

---

#### #74 - Per-chat Working Directory Isolation
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 3 (MicroClaw, Carapace, Moltis)
**LOC Estimate**: 500-600 lines
**Dependencies**: None
**Priority Justification**: Security isolation between chats

**Modes**:
- Shared: All chats share `working_dir/`
- Chat: Each chat isolated to `working_dir/chat/<channel>/<chat_id>`

**Path Guards** (block access to):
- `.ssh/`, `.aws/`, `.env`, `credentials.json`
- System directories (`/etc`, `/usr`, `/bin`)
- Parent directory escapes

---

### Infrastructure

#### #82 - Per-channel Message Chunking and Delivery Queue
**Effort**: 2-3 weeks | **Complexity**: MEDIUM-HIGH | **References**: 3 (MicroClaw, OpenClaw, Moltis)
**LOC Estimate**: 600-800 lines
**Dependencies**: None
**Priority Justification**: Reliable message delivery across platforms

**Chunking Limits**:
- Telegram: 4096 chars
- Discord: 2000 chars
- Slack: 4000 chars
- Feishu: 4000 chars

**Delivery Queue**:
- Persistent queue (SQLite)
- Retry with exponential backoff
- Per-channel concurrency limits
- Dead letter queue for failed messages

---

#### #58 - MCP (Model Context Protocol) Support
**Effort**: 2-3 weeks | **Complexity**: MEDIUM-HIGH | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 800-1000 lines
**Dependencies**: MCP spec compliance
**Priority Justification**: Access to growing MCP ecosystem

**Components**:
- MCP client library
- Server registry and discovery
- Session management
- Tool/resource/prompt registration
- Security sandbox for MCP tools

---

#### #55 - Routines Engine for Automated Tasks
**Effort**: 2-3 weeks | **Complexity**: MEDIUM-HIGH | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 700-900 lines
**Dependencies**: None (uses existing scheduler primitives)
**Priority Justification**: Proactive agent behavior

**Trigger Types**:
- Cron: Time-based scheduling
- Event: File changes, system events
- Webhook: HTTP POST triggers
- Manual: User-initiated

---

## P2: Medium Complexity (3-4 weeks each)
*Valuable enhancements with higher integration needs*

### Advanced Agent Features

#### #57 - Job Scheduler with Parallel Execution
**Effort**: 3-4 weeks | **Complexity**: HIGH | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 900-1200 lines
**Dependencies**: Tokio runtime
**Priority Justification**: Multi-tasking capability

**Features**:
- Concurrent job execution with semaphore limits
- State management (Pending/Running/Completed/Failed)
- Stuck job detection (heartbeat timeout)
- Health monitoring and metrics
- Job cancellation and cleanup

---

#### #68 - Structured JSONL Audit Logging
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 3 (Carapace, Moltis, MicroClaw)
**LOC Estimate**: 500-700 lines
**Dependencies**: None
**Priority Justification**: Compliance and debugging

**Features**:
- 19+ structured event types
- Non-blocking async writes (bounded mpsc channel)
- Automatic rotation at 50MB
- Separate from operational logs
- Query interface for audit trail

---

#### #87 - Agent Event Streaming
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 2 (MicroClaw, OpenClaw)
**LOC Estimate**: 600-700 lines
**Dependencies**: None
**Priority Justification**: Real-time progress visibility

**Event Types**:
- Iteration (count)
- ToolStart (tool name, inputs)
- ToolResult (preview, duration, status, errors)
- TextDelta (streaming chunks)
- FinalResponse

**Consumers**:
- TUI with event display
- Web UI via SSE
- External integrations

---

#### #61 - Multi-agent Routing and Isolation
**Effort**: 3-4 weeks | **Complexity**: HIGH | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 800-1000 lines
**Dependencies**: None
**Priority Justification**: Team collaboration workflows

**Features**:
- Agent profiles (name, personality, tools, workspace)
- Workspace isolation per agent
- Tool restrictions per agent
- Inter-agent communication
- Role-based access control

---

### Web & Messaging

#### #65 - Web Gateway with Real-time Streaming
**Effort**: 3-4 weeks | **Complexity**: HIGH | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 1500-2000 lines
**Dependencies**: WebSocket/SSE support
**Priority Justification**: Complete web API surface

**40+ Endpoints**:
- Chat: POST /chat/send, GET /chat/stream, WS /chat/ws
- Memory: POST /memory/write, GET /memory/search
- Tools: GET /tools/list, POST /tools/execute
- Sessions: GET /sessions/list, POST /sessions/archive
- Config: GET /config, PUT /config
- Health: GET /health, GET /metrics

---

#### #79 - Feishu/Lark Messenger Integration
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 2 (MicroClaw, PicoClaw)
**LOC Estimate**: 600-800 lines
**Dependencies**: Feishu API client
**Priority Justification**: APAC market expansion

**Features**:
- WebSocket (default) and webhook modes
- China (Feishu) and international (Lark) domains
- DM auto-response
- Group @mention triggering
- 4000 char message splitting

---

#### #80 - LINE Messenger Integration
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 2 (PicoClaw, OpenClaw)
**LOC Estimate**: 500-600 lines
**Dependencies**: LINE Messaging API
**Priority Justification**: Japan/Thailand/Taiwan market

**Features**:
- Webhook-based integration
- Text, image, sticker message support
- Group and 1-on-1 chat handling
- Access control lists

---

### Infrastructure & DevEx

#### #75 - Session Archiving with Retention Policies
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 3 (Carapace, MicroClaw, Moltis)
**LOC Estimate**: 600-700 lines
**Dependencies**: None
**Priority Justification**: Disk usage management

**Features**:
- Automatic archiving of old sessions
- Configurable retention (365 days daily, 24 months monthly)
- Compaction metadata tracking
- Session resume with recent N messages
- Historical session access

---

#### #77 - mDNS Service Discovery
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 2 (Carapace, OpenClaw)
**LOC Estimate**: 500-600 lines
**Dependencies**: `mdns-sd = "0.7"`
**Priority Justification**: Zero-config local network pairing

**Features**:
- Service advertisement on gateway start
- Discovery of other RustyClaw instances
- Service type: `_rustyclaw._tcp.local.`
- Three modes: off/minimal/full
- Device pairing and multi-node setup

---

#### #72 - OAuth 2.1 Support for LLM Providers
**Effort**: 3-4 weeks | **Complexity**: HIGH | **References**: 3 (Moltis, PicoClaw, OpenClaw)
**LOC Estimate**: 1000-1200 lines
**Dependencies**: OAuth 2.1 library
**Priority Justification**: Access to free/subscription tier LLMs

**Features**:
- PKCE support (security)
- Device flow for CLI
- Dynamic client registration (RFC 7591)
- OpenID Connect
- Automatic discovery (RFC 8414/9728)
- Token storage and refresh

---

#### #64 - Database Abstraction Layer
**Effort**: 2-3 weeks | **Complexity**: MEDIUM-HIGH | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 800-1000 lines
**Dependencies**: `sqlx` or `libsql`
**Priority Justification**: Backend flexibility

**Supported Backends**:
- PostgreSQL (production)
- libSQL/Turso (embedded + replication)
- SQLite (testing)
- Mock (unit tests)

---

#### #54 - Local Embeddings for Privacy-Preserving Search
**Effort**: 1-2 weeks | **Complexity**: MEDIUM | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 400-600 lines
**Dependencies**: `fastembed-rs`
**Priority Justification**: Privacy, no API costs, offline operation

**Models**:
- `all-MiniLM-L6-v2` (384-dim, ~90MB)
- `BAAI/bge-small-en-v1.5` (384-dim)

**Trade-offs**:
- Pros: No API key, offline, free
- Cons: Slower, less accurate, 90MB download

---

#### #62 - Streaming Responses (Block & Tool Level)
**Effort**: 2-3 weeks | **Complexity**: MEDIUM-HIGH | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 700-900 lines
**Dependencies**: None
**Priority Justification**: Better perceived performance

**Features**:
- Block-level streaming: LLM response chunks
- Tool-level streaming: Progress updates during execution
- Incremental rendering in terminal/web
- Early feedback during long operations

---

## P3: Advanced Features (4-6 weeks each)
*Complex systems requiring significant investment*

### Security & Sandboxing

#### #59 - WASM Sandbox with Capability-based Security
**Effort**: 4-6 weeks | **Complexity**: VERY HIGH | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 1500-2000 lines
**Dependencies**: `wasmtime = "18.0"`, `wasi-common`
**Priority Justification**: Superior sandboxing for untrusted code

**Advantages**:
- 10-100x faster startup than Docker (µs vs 1-2s)
- 10MB vs 512MB+ memory
- Fine-grained capability control
- Cross-platform
- Perfect for Raspberry Pi, embedded devices

**Components**:
1. Wasmtime runtime with fuel metering
2. WASI host functions (file, network, env)
3. Capability-based security (grant specific access)
4. Resource limits (memory, CPU, time)
5. WASM tool SDK for plugin development

---

#### #91 - ClamAV File Scanning
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 1 (AutoGPT)
**LOC Estimate**: 400-600 lines
**Dependencies**: `clamav` daemon
**Priority Justification**: Malware prevention

**Scan Points**:
- `write_file` tool output
- `web_fetch` downloaded files
- Skill installation files

**Implementation**:
- Socket connection to `clamd`
- Async scanning (non-blocking)
- Infected file quarantine
- Agent notification on detection

---

### Intelligence & Automation

#### #60 - Meeting Intelligence Pipeline
**Effort**: 4-5 weeks | **Complexity**: VERY HIGH | **References**: 0 (IronClaw inspired)
**LOC Estimate**: 1200-1500 lines
**Dependencies**: Transcription service, diarization
**Priority Justification**: High-value automation for enterprise

**Pipeline**:
1. Audio ingestion (file watch, Zoom/Meet/Teams APIs)
2. Transcription + diarization (speaker identification)
3. Workspace memory storage
4. LLM processing (action item extraction)
5. Proactive job creation

---

#### #88 - Webhook Trigger System
**Effort**: 3-4 weeks | **Complexity**: HIGH | **References**: 2 (AutoGPT, Carapace)
**LOC Estimate**: 800-1000 lines
**Dependencies**: None
**Priority Justification**: External service integration

**Features**:
- POST /webhook/{agent_id} endpoint
- Webhook payload as agent input context
- HMAC-SHA256 signature verification
- Retry-on-failure semantics
- Webhook secret management

---

#### #89 - Prometheus Metrics Endpoint
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 2 (AutoGPT, Moltis)
**LOC Estimate**: 600-800 lines
**Dependencies**: `prometheus = "0.13"`
**Priority Justification**: Observability for production deployments

**Metrics**:
- Agent execution duration (histogram)
- Block-level latency (histogram)
- Token usage per run (counter)
- Queue depth (gauge)
- Error rates (counter)
- Concurrent agent count (gauge)

---

#### #90 - Granular Per-tool Cost Tracking
**Effort**: 3-4 weeks | **Complexity**: HIGH | **References**: 2 (AutoGPT, OpenClaw)
**LOC Estimate**: 700-900 lines
**Dependencies**: Extends existing cost tracking (#36)
**Priority Justification**: Budget enforcement and cost optimization

**Features**:
- Per-tool cost tracking
- Per-agent budget limits (hard stop)
- Per-user budget limits
- Cost breakdown reports
- Alert on budget threshold
- Cost weight per tool

---

#### #92 - Activity Status Generation
**Effort**: 2-3 weeks | **Complexity**: MEDIUM | **References**: 2 (AutoGPT, OpenClaw)
**LOC Estimate**: 500-600 lines
**Dependencies**: None
**Priority Justification**: Better UX during long operations

**Examples**:
- "Searching codebase..."
- "Analyzing 15 files..."
- "Writing test cases..."
- "Processing 3/5 items..."

**Implementation**:
- Status string per tool/block
- Real-time streaming to UI
- Progress percentage when applicable

---

#### #93 - Nested Agent Composition
**Effort**: 4-5 weeks | **Complexity**: VERY HIGH | **References**: 2 (AutoGPT, Moltis)
**LOC Estimate**: 1000-1300 lines
**Dependencies**: None (extends agent engine)
**Priority Justification**: Advanced workflow orchestration

**Features**:
- Parent-child execution tree
- Agent Executor blocks (run other agents)
- Result aggregation from child agents
- Error propagation and retry
- Up to 3 levels of nesting
- Shared context propagation

---

#### #94 - Multi-provider OAuth Integration Patterns
**Effort**: 4-6 weeks | **Complexity**: VERY HIGH | **References**: 1 (AutoGPT)
**LOC Estimate**: 1500-2000 lines
**Dependencies**: Per-provider OAuth libraries
**Priority Justification**: Third-party service integrations

**Providers** (7+):
- GitHub
- Google Workspace
- Notion
- Jira
- Discord
- Twitter
- Reddit

**Features**:
- Unified OAuth flow (browser redirect)
- Encrypted token storage
- Automatic token refresh
- Per-provider rate limiting
- Available as tool inputs

---

## Recommended Implementation Order

### Phase 1: Foundation (Weeks 1-8)
**Goal**: Security, reliability, and developer experience basics

1. **#86** - Secure credential memory (1 week)
2. **#81** - Retry/backoff engine (1-2 weeks)
3. **#52** - Unified Safety Layer (1-2 weeks)
4. **#83** - Config validation (1-2 weeks)
5. **#84** - Workspace personality files (1 week)
6. **#85** - DuckDuckGo fallback (1 week)

**Deliverable**: Secure, reliable core with great UX

---

### Phase 2: Reliability & Messaging (Weeks 9-14)
**Goal**: Production-ready gateway and messaging

1. **#51** - Multi-provider failover (1-2 weeks)
2. **#73** - Gateway lifecycle management (1-2 weeks)
3. **#70** - CSRF protection (1 week)
4. **#69** - Per-IP rate limiting (1-2 weeks)
5. **#82** - Message chunking/delivery queue (2-3 weeks)

**Deliverable**: Production-ready gateway with reliable messaging

---

### Phase 3: Advanced Agent Capabilities (Weeks 15-22)
**Goal**: Powerful agent features and memory

1. **#53** - Context compaction (1-2 weeks)
2. **#76** - Structured memory with reflector (2-3 weeks)
3. **#66** - Sub-agent spawning (2-3 weeks)
4. **#78** - Lifecycle hook system (2-3 weeks)
5. **#63** - Heartbeat system (1 week)

**Deliverable**: Long-running, intelligent agents with delegation

---

### Phase 4: Security & Intelligence (Weeks 23-30)
**Goal**: Enhanced security and advanced features

1. **#67** - LLM-based injection classifier (2-3 weeks)
2. **#71** - PII redaction (2-3 weeks)
3. **#74** - Per-chat isolation (2-3 weeks)
4. **#87** - Agent event streaming (2-3 weeks)
5. **#55** - Routines engine (2-3 weeks)

**Deliverable**: Secure, intelligent, proactive agents

---

### Phase 5: Ecosystem & Integration (Weeks 31-40)
**Goal**: External integrations and ecosystem growth

1. **#58** - MCP support (2-3 weeks)
2. **#56** - Hybrid search (2-3 weeks)
3. **#54** - Local embeddings (1-2 weeks)
4. **#68** - Audit logging (2-3 weeks)
5. **#65** - Web gateway expansion (3-4 weeks)

**Deliverable**: Rich ecosystem with powerful integrations

---

### Phase 6: Advanced Infrastructure (Weeks 41+)
**Goal**: Sophisticated systems for scale

1. **#57** - Job scheduler (3-4 weeks)
2. **#59** - WASM sandbox (4-6 weeks)
3. **#64** - Database abstraction (2-3 weeks)
4. **#72** - OAuth 2.1 support (3-4 weeks)
5. **#61** - Multi-agent routing (3-4 weeks)
6. **#62** - Streaming responses (2-3 weeks)

**Deliverable**: Scalable, multi-tenant infrastructure

---

### Phase 7: Enterprise & Observability (Weeks 45+)
**Goal**: Enterprise features and operations

1. **#89** - Prometheus metrics (2-3 weeks)
2. **#90** - Per-tool cost tracking (3-4 weeks)
3. **#88** - Webhook triggers (3-4 weeks)
4. **#75** - Session archiving (2-3 weeks)
5. **#77** - mDNS discovery (2-3 weeks)

**Deliverable**: Enterprise-ready operations and monitoring

---

### Phase 8: Advanced Features (Weeks 50+)
**Goal**: Cutting-edge capabilities

1. **#92** - Activity status generation (2-3 weeks)
2. **#93** - Nested agent composition (4-5 weeks)
3. **#60** - Meeting intelligence (4-5 weeks)
4. **#91** - ClamAV scanning (2-3 weeks)
5. **#94** - Multi-provider OAuth (4-6 weeks)
6. **#79** - Feishu/Lark integration (2-3 weeks)
7. **#80** - LINE integration (2-3 weeks)

**Deliverable**: Industry-leading agent platform

---

## Complexity Scoring Methodology

### LOW Complexity (1-2 weeks)
- < 500 LOC
- 0-2 new dependencies
- 0-2 integration points
- Simple testing requirements
- **Examples**: #86 (zeroize), #70 (CSRF), #84 (personality files)

### MEDIUM Complexity (2-3 weeks)
- 500-1000 LOC
- 2-4 new dependencies
- 2-4 integration points
- Moderate testing (unit + integration)
- **Examples**: #81 (retry engine), #67 (LLM classifier), #69 (rate limiting)

### HIGH Complexity (3-4 weeks)
- 1000-1500 LOC
- 4+ new dependencies
- 4+ integration points
- Complex testing (unit + integration + E2E)
- **Examples**: #57 (job scheduler), #65 (web gateway), #72 (OAuth 2.1)

### VERY HIGH Complexity (4-6 weeks)
- 1500+ LOC
- Many dependencies or complex subsystems
- Deep integration with multiple systems
- Extensive testing and edge cases
- **Examples**: #59 (WASM sandbox), #93 (nested agents), #94 (multi-OAuth)

---

## Reference Implementation Summary

| Issue | Ecosystem Count | Projects |
|-------|-----------------|----------|
| **3+ references** | | |
| #66 - Sub-agent spawning | 3 | MicroClaw, Moltis, PicoClaw |
| #68 - Audit logging | 3 | Carapace, Moltis, MicroClaw |
| #69 - Rate limiting | 3 | Carapace, Moltis, MicroClaw |
| #72 - OAuth 2.1 | 3 | Moltis, PicoClaw, OpenClaw |
| #73 - Service lifecycle | 3 | MicroClaw, Moltis, OpenClaw |
| #74 - Per-chat isolation | 3 | MicroClaw, Carapace, Moltis |
| #75 - Session archiving | 3 | Carapace, MicroClaw, Moltis |
| #76 - Structured memory | 3 | MicroClaw, Moltis, AutoGPT |
| #81 - Retry/backoff | 3 | OpenClaw, Moltis, MicroClaw |
| #82 - Message chunking | 3 | MicroClaw, OpenClaw, Moltis |
| #83 - Config validation | 3 | Moltis, OpenClaw, PicoClaw |
| #84 - Personality files | 3 | PicoClaw, MicroClaw, Moltis |
| #85 - DuckDuckGo search | 3 | PicoClaw, Moltis, OpenClaw |
| **2 references** | | |
| #67 - Injection classifier | 2 | Carapace, OpenClaw |
| #70 - CSRF protection | 2 | Carapace, Moltis |
| #71 - PII redaction | 2 | Carapace, Moltis |
| #77 - mDNS discovery | 2 | Carapace, OpenClaw |
| #78 - Lifecycle hooks | 2 | Moltis, OpenClaw |
| #79 - Feishu integration | 2 | MicroClaw, PicoClaw |
| #80 - LINE integration | 2 | PicoClaw, OpenClaw |
| #86 - Secure credentials | 2 | Moltis, Carapace |
| #87 - Event streaming | 2 | MicroClaw, OpenClaw |
| #88 - Webhook triggers | 2 | AutoGPT, Carapace |
| #89 - Prometheus metrics | 2 | AutoGPT, Moltis |
| #90 - Cost tracking | 2 | AutoGPT, OpenClaw |
| #92 - Activity status | 2 | AutoGPT, OpenClaw |
| #93 - Nested agents | 2 | AutoGPT, Moltis |
| **1 reference** | | |
| #91 - ClamAV scanning | 1 | AutoGPT |
| #94 - Multi-OAuth | 1 | AutoGPT |
| **0 references** | | |
| All IronClaw-inspired | 0 | Novel features from IronClaw |

---

## Key Insights

### 1. High-Reference Features Are Lower Risk
Issues with 3+ ecosystem implementations (#66, #68, #69, #72-76, #81-85) have proven architectures to learn from, reducing implementation risk and accelerating development.

### 2. Security Should Come First
Security features (#52, #67, #70, #71, #74, #86, #91) should be prioritized early to establish a strong foundation. Many are quick wins (1-2 weeks).

### 3. Foundation Before Advanced Features
Core reliability (#51, #81, #82) and developer experience (#83, #84, #85) enable faster iteration on advanced features later.

### 4. IronClaw Features Are Innovative But Higher Risk
IronClaw-inspired features (0 ecosystem references) represent novel capabilities but lack reference implementations. These should come after proven patterns are established.

### 5. Parallel Track Opportunities
Independent features can be developed in parallel:
- Security track: #52 → #67 → #70 → #71 → #74 → #86
- Reliability track: #81 → #51 → #69 → #82
- UX track: #83 → #84 → #85 → #63
- Gateway track: #73 → #65 → #88 → #89

### 6. Estimated Timeline
- **Phase 1-2** (Foundation + Reliability): 14 weeks
- **Phase 3-4** (Agent Capabilities + Security): 16 weeks
- **Phase 5-6** (Ecosystem + Infrastructure): 20 weeks
- **Phase 7-8** (Enterprise + Advanced): 16+ weeks
- **Total**: ~66 weeks (~15 months) for full roadmap

### 7. Quick Win Strategy
Focusing on P0 issues (10 features, ~10-14 weeks) delivers:
- Secure credential handling
- Unified security layer
- Multi-provider failover
- Config validation
- Gateway service management
- Personality customization
- Zero-config search
- Proactive monitoring

This creates immediate value while building momentum.

---

## Dependency Graph

### Critical Path
```
#81 (Retry Engine)
  ↓
#51 (LLM Failover)
  ↓
#66 (Sub-agents) + #78 (Hooks)
  ↓
#93 (Nested Agents)
```

### Security Path
```
#52 (Safety Layer)
  ↓
#67 (LLM Classifier)
  ↓
#71 (PII Redaction) + #74 (Isolation)
```

### Memory Path
```
#54 (Local Embeddings)
  ↓
#56 (Hybrid Search)
  ↓
#76 (Structured Memory)
```

### Gateway Path
```
#73 (Service Lifecycle)
  ↓
#70 (CSRF) + #69 (Rate Limit)
  ↓
#65 (Web Gateway)
  ↓
#88 (Webhooks) + #89 (Metrics)
```

---

## Risk Assessment

### High Risk (Proceed with Caution)
- **#59** - WASM sandbox: Complex integration, Wasmtime API surface
- **#93** - Nested agents: Circular dependency risks, complex state management
- **#94** - Multi-OAuth: 7+ providers, each with quirks
- **#60** - Meeting pipeline: External API dependencies, audio processing complexity

### Medium Risk
- **#57** - Job scheduler: Concurrency bugs, race conditions
- **#65** - Web gateway: Large API surface, 40+ endpoints
- **#72** - OAuth 2.1: Security-sensitive, RFC compliance
- **#78** - Lifecycle hooks: Plugin security isolation

### Low Risk
- **#81-86** - Well-documented patterns, multiple references
- **#83-85** - Simple implementations, clear scope
- **#52** - Refactoring existing code, not net-new

---

## Success Metrics

### Phase 1 Success Criteria
- Zero memory leaks from credentials (valgrind/miri clean)
- < 5ms retry decision latency
- 95% of config errors caught with actionable messages
- Personality files load < 100ms

### Phase 2 Success Criteria
- < 30s failover time on provider outage
- Gateway survives 1000 req/s load test
- Zero message loss in delivery queue
- CSRF protection passes OWASP test suite

### Phase 3 Success Criteria
- Context compaction preserves 90%+ semantic meaning
- Sub-agents spawn in < 200ms
- Memory reflector extracts facts with 85%+ precision
- Hooks execute with < 50ms overhead

### Phase 4 Success Criteria
- LLM classifier achieves 95%+ precision on injection detection
- PII redaction catches 99%+ of common patterns
- Event streaming latency < 100ms
- Routines trigger within 1s of schedule

---

## Conclusion

This roadmap prioritizes velocity through strategic sequencing:

1. **Quick Wins First** (P0): Establish security and reliability foundation
2. **High-Value Features** (P1): Core agent capabilities with proven patterns
3. **Medium Complexity** (P2): Valuable enhancements with higher integration
4. **Advanced Systems** (P3): Sophisticated features for scale

**Key Strategy**: Features with 3+ ecosystem references should be prioritized for lower risk and faster implementation. IronClaw-inspired novel features should come later after core patterns are established.

**Recommended Start**: Begin with #86 (secure credentials) → #81 (retry engine) → #52 (safety layer) → #83 (config validation) for maximum early impact.

**Estimated Full Completion**: ~15 months with parallel tracks, or ~20 months linear development.
