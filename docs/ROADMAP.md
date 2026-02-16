# RustyClaw Development Roadmap
## Feature Adoption from IronClaw Analysis

This roadmap outlines the strategic adoption of features from IronClaw and other improvements to enhance RustyClaw's capabilities while maintaining its lightweight, Rust-native advantages.

---

## 🎯 Vision

Transform RustyClaw into a best-in-class agentic AI runtime with:
- **Superior security** via multi-layer sandboxing (Landlock+bwrap, WASM, Docker)
- **Enhanced intelligence** through hybrid search and local embeddings
- **Proactive automation** via routines engine and heartbeat system
- **Reliability** through multi-provider failover and context compaction
- **Extensibility** via MCP protocol and WASM plugin system

---

## 📊 Current State (February 2026)

### ✅ Completed
- Landlock+Bubblewrap combined sandbox (defense-in-depth)
- Docker container sandboxing (cross-platform)
- Basic memory search (keyword-based)
- Multi-provider support (no automatic failover yet)
- SSRF and prompt injection defense (basic)
- Browser automation with CDP
- Multiple messenger channels (6+)
- Secrets vault with TOTP/WebAuthn

### 🔄 In Progress
- Profile management for browser automation

### ⚠️ Gaps Identified
- No scheduled/automated tasks (routines)
- No hybrid search (vector + BM25)
- Single-provider dependency (no failover)
- Context window management
- No WASM sandboxing
- Limited memory system

---

## 🚀 Phase 1: Quick Wins (2-3 weeks)
**Timeline**: March 2026
**Focus**: High-impact, low-effort improvements

### 1.1 Multi-Provider Failover ⭐⭐⭐⭐
**Priority**: HIGH | **Effort**: 1-2 weeks

**What**: Automatic failover across multiple LLM providers
- Try providers in sequence on retryable errors
- Cost tracking reflects actual serving provider
- Configurable provider order

**Benefits**:
- Improved reliability (no single point of failure)
- Better handling of rate limits
- Cost optimization

**Implementation**:
```rust
pub struct FailoverProvider {
    providers: Vec<Arc<dyn LlmProvider>>,
    last_used: AtomicUsize,
}
```

**Acceptance Criteria**:
- [ ] Wrapper provider implementation
- [ ] Retryable vs non-retryable error classification
- [ ] Configuration via `config.toml`
- [ ] Cost tracking per actual provider
- [ ] Tests for failover scenarios

---

### 1.2 Safety Layer Consolidation ⭐⭐⭐⭐⭐ ✅
**Priority**: VERY HIGH | **Effort**: 1-2 weeks | **Status**: COMPLETED

**What**: Unified security defense with 4 components
- Sanitizer: Pattern-based content cleaning
- Validator: Input validation with rules
- Policy engine: Warn/Block/Sanitize/Ignore actions
- Leak detector: Credential exfiltration prevention

**Benefits**:
- Comprehensive defense-in-depth
- Better maintainability (single layer)
- Configurable sensitivity

**Current State**: ~~Separate SSRF + prompt guard modules~~ **COMPLETED**
**Target State**: Unified `SafetyLayer` struct ✅

**Acceptance Criteria**:
- [x] Consolidate existing SSRF and prompt guard
- [x] Add leak detector with credential patterns (API keys, passwords, tokens, private keys, PII)
- [x] Policy configuration via `config.toml`
- [x] Severity levels (ignore/warn/block/sanitize)
- [x] Tests for all defense categories (9 tests passing)

---

### 1.3 Context Compaction ⭐⭐⭐⭐
**Priority**: MEDIUM-HIGH | **Effort**: 1-2 weeks

**What**: Automatic conversation context management
- Summarize (LLM-generated executive summary)
- Truncate (drop oldest messages)
- MoveToWorkspace (archive to daily logs)

**Benefits**:
- Prevents context window exhaustion
- Enables indefinitely long conversations
- Historical context preservation

**Acceptance Criteria**:
- [ ] Token counting for context monitoring
- [ ] Compaction at 75% threshold
- [ ] LLM summarization strategy
- [ ] Workspace archiving strategy
- [ ] Configuration for strategy selection

---

### 1.4 Local Embeddings (Privacy Mode) ⭐⭐⭐
**Priority**: MEDIUM | **Effort**: 1-2 weeks

**What**: Run embedding models locally
- `fastembed-rs` integration
- Models: `all-MiniLM-L6-v2` (384-dim)
- No API key required, offline-capable

**Benefits**:
- Privacy-preserving (no data sent to cloud)
- No API costs
- Offline operation

**Trade-offs**:
- Lower quality vs OpenAI (384-dim vs 1536)
- CPU-bound execution
- ~90MB model download

**Acceptance Criteria**:
- [ ] `EmbeddingProvider` implementation for local models
- [ ] Model download on first use
- [ ] Configuration: `embedding_provider = "local"`
- [ ] Fallback to OpenAI if local fails
- [ ] Performance benchmarking

---

## 🔧 Phase 2: Core Features (4-6 weeks)
**Timeline**: April-May 2026
**Focus**: Fundamental capability additions

### 2.1 Routines Engine (Automation) ⭐⭐⭐⭐⭐
**Priority**: VERY HIGH | **Effort**: 2-3 weeks

**What**: Scheduled and event-driven automation
- Cron triggers (`"0 9 * * MON-FRI"`)
- Event triggers (regex pattern matching)
- Webhook triggers (external integrations)
- Manual execution

**Use Cases**:
- Periodic backups to workspace
- Monitor RSS feeds/GitHub repos
- Daily standup reminders
- Auto-cleanup of old sessions

**Architecture**:
```
RoutineEngine → Cron Ticker + Event Matcher
              ↓
          Scheduler → Execute Routine Prompt
```

**Acceptance Criteria**:
- [ ] Database schema for routines storage
- [ ] Cron expression parser (`cron` crate)
- [ ] Event pattern matcher (regex)
- [ ] Webhook endpoint with HMAC validation
- [ ] Guardrails (max failures, cooldown)
- [ ] CLI commands: `routine list/create/delete/run`

---

### 2.2 Hybrid Search (BM25 + Vector) ⭐⭐⭐⭐
**Priority**: HIGH | **Effort**: 2-3 weeks

**What**: Combined full-text and semantic search
- BM25 full-text search
- Vector similarity search
- Reciprocal Rank Fusion (RRF) algorithm
- Document chunking with overlap

**Benefits**:
- Better context retrieval (intent + keywords)
- RRF combines strengths of both methods
- Handles both specific queries and semantic exploration

**Current**: Keyword-only search
**Target**: Hybrid search with configurable weights

**Acceptance Criteria**:
- [ ] BM25 implementation or integration
- [ ] Vector search via embeddings
- [ ] RRF algorithm for result fusion
- [ ] Configurable weights (vector_weight, bm25_weight)
- [ ] Chunking with configurable size/overlap
- [ ] Performance benchmarks vs current keyword search

---

### 2.3 Job Scheduler (Parallel Execution) ⭐⭐⭐⭐
**Priority**: HIGH | **Effort**: 3-4 weeks

**What**: Concurrent multi-job execution
- Parallel job scheduling with state management
- Stuck job detection and recovery
- Max parallel jobs enforcement
- Per-job context isolation

**Benefits**:
- Multi-tasking (research while monitoring feeds)
- Background jobs don't block user interaction
- Better CPU utilization on multi-core

**Current**: Linear single-job execution
**Target**: Parallel job orchestration

**Architecture**:
```rust
pub struct Scheduler {
    jobs: Arc<RwLock<HashMap<Uuid, ScheduledJob>>>,
    subtasks: Arc<RwLock<HashMap<Uuid, ScheduledSubtask>>>,
}

pub enum JobState {
    Pending → InProgress → Completed
                        ↘ Failed
}
```

**Acceptance Criteria**:
- [ ] Job state machine with valid transitions
- [ ] Concurrent HashMap with RwLock
- [ ] Health check for stuck jobs
- [ ] Max parallel jobs limit (configurable)
- [ ] Job cancellation support
- [ ] CLI: `job list/status/cancel`

---

### 2.4 MCP (Model Context Protocol) Support ⭐⭐⭐
**Priority**: MEDIUM-HIGH | **Effort**: 2-3 weeks

**What**: Standards-based tool integration
- Local unauthenticated MCP servers
- Hosted servers with OAuth
- Tool registry integration
- Auto-discovery from MCP registry

**Benefits**:
- Access to growing MCP ecosystem (Anthropic-backed)
- Community-driven extensions
- Reduced maintenance burden

**Acceptance Criteria**:
- [ ] MCP protocol client implementation
- [ ] Local server connection support
- [ ] OAuth flow for hosted servers
- [ ] Tool wrapper generation from MCP capabilities
- [ ] Registry integration for discovery
- [ ] CLI: `mcp list/connect/disconnect`

---

## 🎨 Phase 3: Advanced Features (8-12 weeks)
**Timeline**: June-August 2026
**Focus**: Cutting-edge capabilities

### 3.1 WASM Sandbox (Security + Performance) ⭐⭐⭐⭐⭐
**Priority**: VERY HIGH | **Effort**: 4-6 weeks

**What**: WebAssembly-based tool sandboxing
- Wasmtime runtime with capability-based security
- Fuel metering (CPU limits)
- Memory limits (10MB default)
- Credential injection at host boundary
- Leak detection on outputs

**Benefits vs Docker**:
- 10-100x faster startup (µs vs 1-2s)
- 10MB memory vs 512MB+ for containers
- Fine-grained capability control
- Perfect for Raspberry Pi / embedded

**Architecture**:
```
WASM Tool → Host Function → Allowlist → Credential → Execute
                           Validator    Injector
           ← Leak Detector ← Response (sanitized)
```

**Acceptance Criteria**:
- [ ] Wasmtime integration (`wasmtime = "28"`)
- [ ] Capability schema definition
- [ ] Host function implementations
- [ ] Fuel metering configuration
- [ ] Memory limit enforcement
- [ ] BLAKE3 hash verification for WASM modules
- [ ] Tool ABI redesign for WASM component model
- [ ] Migration guide for existing tools

---

### 3.2 Meeting Intelligence Pipeline ⭐⭐⭐
**Priority**: MEDIUM | **Effort**: 4-5 weeks

**What**: End-to-end meeting processing
- Audio file watch (`~/meetings/`)
- Integration with Zoom/Meet/Teams APIs
- Transcription with speaker diarization
- Action item extraction
- Automatic task creation
- Proactive follow-up

**Architecture**:
```
Audio Ingestion → Transcription + Diarization →
Workspace Memory → LLM Processing → Action Items →
Proactive Job Creation
```

**Acceptance Criteria**:
- [ ] File watch for audio/video files
- [ ] Transcription provider with diarization
- [ ] Structured transcript storage in workspace
- [ ] LLM-based action item extraction
- [ ] Automatic routine creation for follow-ups
- [ ] Meeting context in search results

---

### 3.3 Multi-Agent Routing ⭐⭐⭐
**Priority**: MEDIUM | **Effort**: 3-4 weeks

**What**: Multiple specialized agents
- Agent registry with profiles
- Workspace isolation per agent
- Tool access restrictions per agent
- Inter-agent communication
- Message routing to target agent

**Use Cases**:
- Specialized agents (research, coding, ops)
- Team collaboration workflows
- Role-based access control

**Acceptance Criteria**:
- [ ] Agent profile configuration
- [ ] Workspace scoping by agent_id
- [ ] Tool registry per agent
- [ ] Message routing logic
- [ ] Inter-agent message protocol
- [ ] CLI: `agent list/create/switch`

---

### 3.4 Streaming (Block & Tool Level) ⭐⭐⭐
**Priority**: MEDIUM | **Effort**: 2-3 weeks

**What**: Real-time response streaming
- Block-level streaming (LLM chunks)
- Tool execution progress streaming
- WebSocket/SSE for web gateway
- Incremental rendering

**Benefits**:
- Better perceived performance
- Real-time feedback during long operations
- Improved user experience

**Acceptance Criteria**:
- [ ] `LlmProvider::complete_streaming()` method
- [ ] Stream<Item = String> response type
- [ ] Agent loop accumulates and forwards chunks
- [ ] StatusUpdate::StreamChunk handling
- [ ] WebSocket/SSE broadcasting
- [ ] Tool progress reporting

---

## 📋 Supporting Features

### Database Abstraction
**Effort**: 2-3 weeks

- Trait-based persistence layer
- PostgreSQL implementation (existing)
- libSQL/Turso implementation (new)
- Feature flags for backend selection

### Web Gateway Expansion
**Effort**: 3-4 weeks

- Real-time SSE/WebSocket streaming
- Comprehensive REST API (40+ endpoints)
- Rate limiting (sliding window)
- Bearer token authentication
- Frontend development (Vue/React)

### Heartbeat System
**Effort**: 1 week

- Periodic background execution (configurable interval)
- Reads HEARTBEAT.md checklist from workspace
- Silent execution if no action needed
- User notification only when required

---

## 🎯 Success Metrics

### Performance
- Startup time: <50ms (maintain)
- Memory usage: <20MB base (current: ~15MB)
- WASM tool execution: <10ms startup vs Docker 1-2s

### Security
- Zero credential leaks in production
- 100% sandbox escape resistance (kernel-enforced modes)
- All external inputs validated via SafetyLayer

### Reliability
- 99.9% uptime with multi-provider failover
- Context window: unlimited with compaction
- Job success rate: >95% with scheduler

### Extensibility
- 50+ MCP tools available via ecosystem
- 10+ WASM plugins in first 6 months
- Community contribution rate: 5+ PRs/month

---

## 🔄 Iteration Process

### Weekly Cycle
1. **Monday**: Review roadmap priorities
2. **Wed**: Mid-week progress check
3. **Friday**: Demo + retrospective
4. **Continuous**: GitHub issue updates

### Release Cadence
- **Minor releases**: Every 2 weeks (bug fixes, small features)
- **Major releases**: Every 6 weeks (roadmap phase completion)
- **Security releases**: As needed (immediate)

---

## 📚 References

- **IronClaw Analysis**: Comprehensive feature review (46,447 lines, 168 files)
- **GitHub Issues**: 50 issues reviewed from IronClaw project
- **PARITY_PLAN.md**: Feature comparison across 7 AI runtimes
- **FEATURE_PARITY.md**: Detailed feature tracking

---

## 🤝 Contributing

Features in this roadmap are tracked as GitHub issues with labels:
- `enhancement`: New features
- `p1-critical`, `p2-high`, `p3-medium`: Priority levels
- `quick-win`: Can be completed in 1-2 weeks
- `ironclaw-inspired`: Borrowed from IronClaw analysis

See individual issues for implementation details and acceptance criteria.

---

**Last Updated**: February 16, 2026
**Next Review**: March 1, 2026
