# RustyClaw Feature Prioritization Matrix
## Issues #51-#94 Visual Analysis

**Generated**: 2026-02-16

---

## Priority Matrix: Complexity vs References

```
High Refs (3+)  │  #81 #83 #84 #85    │  #66 #68 #69 #74    │  #72 #75 #76 #82    │
                │  P0 QUICK WINS      │  P1 HIGH VALUE      │  P2 MEDIUM          │
                │  ■■■■                │  ■■■■               │  ■■■■               │
────────────────┼─────────────────────┼─────────────────────┼─────────────────────┤
Med Refs (2)    │  #70 #86            │  #67 #71 #78        │  #77 #79 #80 #87    │  #88 #89 #90 #92 #93
                │  P0 QUICK WINS      │  P1 HIGH VALUE      │  P2 MEDIUM          │  P3 ADVANCED
                │  ■■                 │  ■■■                │  ■■■■               │  ■■■■■
────────────────┼─────────────────────┼─────────────────────┼─────────────────────┤
Low Refs (0-1)  │  #51 #52 #63 #73    │  #53 #55 #56 #58    │  #54 #57 #61 #62    │  #59 #60 #91 #94
                │  P0 QUICK WINS      │  P1 HIGH VALUE      │  #64 #65            │  P3 ADVANCED
                │  ■■■■               │  ■■■■               │  P2 MEDIUM          │  ■■■■
                │                     │                     │  ■■■■■■             │
────────────────┴─────────────────────┴─────────────────────┴─────────────────────┴──────────────────
                     LOW (1-2w)            MEDIUM (2-3w)           HIGH (3-4w)          VERY HIGH (4-6w)
                                                COMPLEXITY
```

**Legend**:
- **High Refs (3+)**: Proven implementations, low risk
- **Med Refs (2)**: Some examples, moderate risk
- **Low Refs (0-1)**: Novel/IronClaw-inspired, higher risk
- **■**: Issue count in quadrant

---

## Effort vs Impact Matrix

```
High Impact     │  #52 #81 #86        │  #51 #66 #67 #76    │  #58 #72 #78 #82    │  #59 #93
                │  P0 ⚡️              │  P1 🔥              │  P2 ⭐️             │  P3 🚀
                │  Security + Core    │  Agent + Memory     │  Integration        │  Advanced
────────────────┼─────────────────────┼─────────────────────┼─────────────────────┤
Med Impact      │  #63 #70 #83        │  #53 #55 #69 #71    │  #57 #61 #64 #65    │  #60 #88 #89
                │  #84 #85            │  #74                │  #68 #75 #77 #87    │  #90 #94
                │  P0 ⚡️              │  P1 🔥              │  P2 ⭐️             │  P3 🚀
                │  DevEx              │  Security + Ops     │  Infrastructure     │  Enterprise
────────────────┼─────────────────────┼─────────────────────┼─────────────────────┤
Lower Impact    │                     │  #56 #54            │  #62 #79 #80        │  #91 #92
                │                     │  P1 🔥              │  P2 ⭐️             │  P3 🚀
                │                     │  Search/Embeddings  │  Messaging/Stream   │  Specialized
────────────────┴─────────────────────┴─────────────────────┴─────────────────────┴──────────────────
                     LOW (1-2w)            MEDIUM (2-3w)           HIGH (3-4w)          VERY HIGH (4-6w)
                                                EFFORT
```

**Symbol Key**:
- ⚡️ **P0 Quick Wins**: High impact, low effort - do first!
- 🔥 **P1 High Value**: High impact, medium effort - critical features
- ⭐️ **P2 Medium**: Medium impact/effort - valuable enhancements
- 🚀 **P3 Advanced**: Lower impact or very high effort - nice to have

---

## Implementation Sequence Flowchart

```
                                    START
                                      │
                    ┌─────────────────┼─────────────────┐
                    │                 │                 │
              SECURITY            RELIABILITY        DEVEX
                    │                 │                 │
                   #86               #81               #83
            (Secure Creds)      (Retry Engine)   (Config Valid)
                    │                 │                 │
                   #52               #51               #84
            (Safety Layer)       (Failover)      (Personality)
                    │                 │                 │
                   #70               #69               #85
              (CSRF Protect)    (Rate Limit)     (DuckDuckGo)
                    │                 │                 │
                    └─────────────────┼─────────────────┘
                                      │
                            ┌─────────┴─────────┐
                            │                   │
                      AGENT FEATURES      GATEWAY & OPS
                            │                   │
                           #53                 #73
                    (Compaction)        (Service Lifecycle)
                            │                   │
                           #76                 #82
                  (Structured Memory)    (Message Queue)
                            │                   │
                           #66                 #65
                      (Sub-agents)        (Web Gateway)
                            │                   │
                           #78                 #88
                    (Lifecycle Hooks)      (Webhooks)
                            │                   │
                           #55                 #89
                    (Routines Engine)      (Prometheus)
                            │                   │
                            └─────────┬─────────┘
                                      │
                              ADVANCED FEATURES
                                      │
                        ┌─────────────┼─────────────┐
                        │             │             │
                       #59           #93           #60
                  (WASM Sandbox)  (Nested)    (Meeting Intel)
                        │             │             │
                        └─────────────┴─────────────┘
                                      │
                                     END
```

---

## Feature Categories Analysis

### Security Features (10 issues)

| Priority | Issue | Feature | Effort | Impact |
|----------|-------|---------|--------|--------|
| ⚡️ P0 | #52 | Unified Safety Layer | 1-2w | Critical |
| ⚡️ P0 | #70 | CSRF Protection | 1w | High |
| ⚡️ P0 | #86 | Secure Credentials | 1w | High |
| 🔥 P1 | #67 | LLM Injection Classifier | 2-3w | High |
| 🔥 P1 | #69 | Per-IP Rate Limiting | 1-2w | High |
| 🔥 P1 | #71 | PII Redaction | 2-3w | High |
| 🔥 P1 | #74 | Per-chat Isolation | 2-3w | High |
| ⭐️ P2 | #68 | Audit Logging | 2-3w | Medium |
| 🚀 P3 | #59 | WASM Sandbox | 4-6w | Very High |
| 🚀 P3 | #91 | ClamAV Scanning | 2-3w | Low |

**Total Security Effort**: 18-27 weeks

---

### Reliability Features (7 issues)

| Priority | Issue | Feature | Effort | Impact |
|----------|-------|---------|--------|--------|
| ⚡️ P0 | #81 | Retry/Backoff Engine | 1-2w | Critical |
| ⚡️ P0 | #51 | Multi-provider Failover | 1-2w | Critical |
| 🔥 P1 | #82 | Message Chunking/Queue | 2-3w | High |
| ⭐️ P2 | #57 | Job Scheduler | 3-4w | Medium |
| ⭐️ P2 | #64 | Database Abstraction | 2-3w | Medium |
| ⭐️ P2 | #75 | Session Archiving | 2-3w | Low |

**Total Reliability Effort**: 12-19 weeks

---

### Agent Capabilities (11 issues)

| Priority | Issue | Feature | Effort | Impact |
|----------|-------|---------|--------|--------|
| 🔥 P1 | #53 | Context Compaction | 1-2w | High |
| 🔥 P1 | #76 | Structured Memory | 2-3w | Very High |
| 🔥 P1 | #56 | Hybrid Search | 2-3w | High |
| 🔥 P1 | #66 | Sub-agent Spawning | 2-3w | High |
| 🔥 P1 | #78 | Lifecycle Hooks | 2-3w | High |
| 🔥 P1 | #55 | Routines Engine | 2-3w | High |
| ⭐️ P2 | #61 | Multi-agent Routing | 3-4w | Medium |
| ⭐️ P2 | #54 | Local Embeddings | 1-2w | Medium |
| ⭐️ P2 | #87 | Event Streaming | 2-3w | Medium |
| 🚀 P3 | #93 | Nested Agents | 4-5w | High |
| 🚀 P3 | #60 | Meeting Intelligence | 4-5w | Medium |

**Total Agent Effort**: 24-35 weeks

---

### Infrastructure (8 issues)

| Priority | Issue | Feature | Effort | Impact |
|----------|-------|---------|--------|--------|
| ⚡️ P0 | #73 | Gateway Lifecycle | 1-2w | High |
| 🔥 P1 | #58 | MCP Support | 2-3w | High |
| ⭐️ P2 | #65 | Web Gateway | 3-4w | High |
| ⭐️ P2 | #72 | OAuth 2.1 | 3-4w | Medium |
| ⭐️ P2 | #77 | mDNS Discovery | 2-3w | Low |
| 🚀 P3 | #88 | Webhook Triggers | 3-4w | Medium |
| 🚀 P3 | #89 | Prometheus Metrics | 2-3w | Medium |
| 🚀 P3 | #94 | Multi-provider OAuth | 4-6w | Low |

**Total Infrastructure Effort**: 21-31 weeks

---

### Developer Experience (5 issues)

| Priority | Issue | Feature | Effort | Impact |
|----------|-------|---------|--------|--------|
| ⚡️ P0 | #83 | Config Validation | 1-2w | High |
| ⚡️ P0 | #84 | Personality Files | 1w | High |
| ⚡️ P0 | #85 | DuckDuckGo Fallback | 1w | High |
| ⚡️ P0 | #63 | Heartbeat Monitoring | 1w | Medium |
| 🚀 P3 | #92 | Activity Status | 2-3w | Low |

**Total DevEx Effort**: 6-9 weeks

---

### Messaging/Channels (3 issues)

| Priority | Issue | Feature | Effort | Impact |
|----------|-------|---------|--------|--------|
| ⭐️ P2 | #79 | Feishu/Lark | 2-3w | Low |
| ⭐️ P2 | #80 | LINE | 2-3w | Low |
| ⭐️ P2 | #62 | Streaming Responses | 2-3w | Medium |

**Total Messaging Effort**: 6-9 weeks

---

### Operations (1 issue)

| Priority | Issue | Feature | Effort | Impact |
|----------|-------|---------|--------|--------|
| 🚀 P3 | #90 | Per-tool Cost Tracking | 3-4w | Medium |

**Total Ops Effort**: 3-4 weeks

---

## Complexity Score Breakdown

Each issue scored on 5 factors (0-3 points each, max 15):

| Issue | LOC | Deps | Integration | Testing | Async | Total | Complexity |
|-------|-----|------|-------------|---------|-------|-------|------------|
| #86 | 1 | 1 | 0 | 1 | 0 | **3** | LOW |
| #70 | 1 | 0 | 0 | 1 | 0 | **2** | LOW |
| #84 | 1 | 0 | 0 | 1 | 0 | **2** | LOW |
| #85 | 1 | 0 | 0 | 1 | 0 | **2** | LOW |
| #63 | 1 | 0 | 0 | 1 | 1 | **3** | LOW |
| #52 | 2 | 0 | 1 | 2 | 0 | **5** | MEDIUM |
| #81 | 2 | 0 | 1 | 2 | 1 | **6** | MEDIUM |
| #83 | 2 | 1 | 0 | 2 | 0 | **5** | MEDIUM |
| #51 | 2 | 1 | 1 | 2 | 0 | **6** | MEDIUM |
| #73 | 2 | 0 | 1 | 2 | 0 | **5** | MEDIUM |
| #53 | 2 | 0 | 1 | 2 | 0 | **5** | MEDIUM |
| #67 | 2 | 0 | 1 | 2 | 0 | **5** | MEDIUM |
| #69 | 2 | 0 | 1 | 2 | 0 | **5** | MEDIUM |
| #71 | 2 | 0 | 2 | 2 | 0 | **6** | MEDIUM |
| #74 | 2 | 0 | 2 | 2 | 0 | **6** | MEDIUM |
| #54 | 2 | 2 | 0 | 1 | 0 | **5** | MEDIUM |
| #66 | 2 | 0 | 2 | 2 | 1 | **7** | MEDIUM-HIGH |
| #76 | 3 | 1 | 1 | 2 | 0 | **7** | MEDIUM-HIGH |
| #56 | 3 | 2 | 2 | 2 | 1 | **10** | HIGH |
| #78 | 3 | 0 | 3 | 2 | 1 | **9** | HIGH |
| #58 | 3 | 1 | 3 | 2 | 1 | **10** | HIGH |
| #55 | 3 | 1 | 2 | 2 | 0 | **8** | MEDIUM-HIGH |
| #82 | 2 | 0 | 2 | 2 | 1 | **7** | MEDIUM-HIGH |
| #57 | 3 | 0 | 2 | 3 | 2 | **10** | HIGH |
| #68 | 2 | 0 | 2 | 2 | 1 | **7** | MEDIUM-HIGH |
| #87 | 2 | 0 | 2 | 2 | 1 | **7** | MEDIUM-HIGH |
| #61 | 3 | 0 | 2 | 2 | 0 | **7** | MEDIUM-HIGH |
| #65 | 3 | 1 | 3 | 3 | 2 | **12** | HIGH |
| #72 | 3 | 2 | 2 | 3 | 1 | **11** | HIGH |
| #64 | 3 | 2 | 2 | 2 | 1 | **10** | HIGH |
| #75 | 2 | 0 | 2 | 2 | 0 | **6** | MEDIUM |
| #77 | 2 | 2 | 2 | 2 | 0 | **8** | MEDIUM-HIGH |
| #79 | 2 | 1 | 2 | 2 | 0 | **7** | MEDIUM-HIGH |
| #80 | 2 | 1 | 2 | 2 | 0 | **7** | MEDIUM-HIGH |
| #62 | 3 | 0 | 2 | 2 | 2 | **9** | HIGH |
| #59 | 3 | 3 | 3 | 3 | 2 | **14** | VERY HIGH |
| #60 | 3 | 2 | 3 | 3 | 2 | **13** | VERY HIGH |
| #88 | 3 | 0 | 3 | 2 | 2 | **10** | HIGH |
| #89 | 2 | 2 | 2 | 2 | 0 | **8** | MEDIUM-HIGH |
| #90 | 3 | 0 | 2 | 2 | 1 | **8** | MEDIUM-HIGH |
| #91 | 2 | 2 | 2 | 2 | 0 | **8** | MEDIUM-HIGH |
| #92 | 2 | 1 | 1 | 2 | 1 | **7** | MEDIUM-HIGH |
| #93 | 3 | 0 | 3 | 3 | 2 | **11** | HIGH |
| #94 | 3 | 2 | 3 | 3 | 1 | **12** | HIGH |

**Scoring**:
- **LOC**: 0=<300, 1=300-600, 2=600-1200, 3=>1200
- **Deps**: 0=none, 1=1-2, 2=3-4, 3=5+
- **Integration**: 0=standalone, 1=1-2 points, 2=3-4 points, 3=5+ points
- **Testing**: 0=minimal, 1=unit, 2=unit+integration, 3=unit+integration+e2e
- **Async**: 0=sync, 1=basic async, 2=complex async

**Complexity Tiers**:
- 0-4: LOW
- 5-7: MEDIUM
- 8-10: HIGH
- 11+: VERY HIGH

---

## Quick Reference: First 10 Features to Implement

| Week | Issue | Feature | Why |
|------|-------|---------|-----|
| **1** | #86 | Secure Credentials | Security foundation, simple |
| **2-3** | #81 | Retry/Backoff Engine | Reliability foundation, reusable |
| **4-5** | #52 | Unified Safety Layer | Security consolidation |
| **6-7** | #83 | Config Validation | UX improvement, catches errors early |
| **8** | #84 | Personality Files | Easy customization |
| **9** | #85 | DuckDuckGo Fallback | Zero-config search |
| **10** | #70 | CSRF Protection | Gateway security |
| **11-12** | #51 | Multi-provider Failover | Builds on #81, critical reliability |
| **13-14** | #73 | Gateway Lifecycle | Production deployment |
| **15** | #63 | Heartbeat Monitoring | Proactive monitoring |

**Total**: 15 weeks, **10 features shipped**, solid foundation established

---

## Velocity Estimation

### Conservative (1 feature/week for medium complexity)
- P0 (10 issues): 10-14 weeks
- P1 (12 issues): 22-32 weeks
- P2 (14 issues): 30-41 weeks
- P3 (8 issues): 26-38 weeks
- **Total**: 88-125 weeks (~20-28 months)

### Moderate (1.5 features/week average)
- P0: 7-9 weeks
- P1: 15-21 weeks
- P2: 20-27 weeks
- P3: 17-25 weeks
- **Total**: 59-82 weeks (~14-19 months)

### Aggressive (2 features/week with parallelism)
- P0: 5-7 weeks
- P1: 11-16 weeks
- P2: 15-21 weeks
- P3: 13-19 weeks
- **Total**: 44-63 weeks (~10-15 months)

**Recommended**: Moderate pace with 2-3 engineers in parallel tracks

---

## Risk-Adjusted Priority Scores

Formula: `Score = (Impact × References) / (Effort × Risk)`

| Rank | Issue | Score | Rationale |
|------|-------|-------|-----------|
| 1 | **#86** | 12.0 | High impact, 2 refs, 1 week, low risk |
| 2 | **#84** | 11.5 | High impact, 3 refs, 1 week, low risk |
| 3 | **#85** | 11.5 | High impact, 3 refs, 1 week, low risk |
| 4 | **#81** | 10.5 | Critical impact, 3 refs, 1-2 weeks |
| 5 | **#70** | 10.0 | High impact, 2 refs, 1 week |
| 6 | **#83** | 9.5 | High impact, 3 refs, 1-2 weeks |
| 7 | **#52** | 9.0 | Critical impact, refactor, 1-2 weeks |
| 8 | **#63** | 8.5 | Medium impact, simple, 1 week |
| 9 | **#51** | 8.0 | Critical impact, depends on #81 |
| 10 | **#73** | 7.5 | High impact, 3 refs, essential |
| 11 | **#69** | 7.0 | High impact, 3 refs, 1-2 weeks |
| 12 | **#66** | 6.5 | High impact, 3 refs, moderate complexity |
| 13 | **#76** | 6.5 | Very high impact, 3 refs, 2-3 weeks |
| 14 | **#82** | 6.0 | High impact, 3 refs, reliability |
| 15 | **#67** | 5.5 | High impact, 2 refs, security |

Top 15 features represent the highest ROI for development effort.

---

## Decision Matrix: Build, Buy, or Skip

### Build (Core Differentiators)
- #52 (Safety Layer) - RustyClaw-specific security
- #51 (Failover) - Critical reliability
- #76 (Structured Memory) - Intelligence differentiation
- #66 (Sub-agents) - Core agent capability
- #59 (WASM Sandbox) - Unique security approach

### Build (Quick Wins)
- #86 (Secure creds) - Simple wrapper pattern
- #84 (Personality) - File loading only
- #85 (DuckDuckGo) - HTML parsing
- #70 (CSRF) - Standard pattern
- #63 (Heartbeat) - Cron-like feature

### Consider Library/Crate
- #81 (Retry) - Could use `backoff` crate
- #69 (Rate limit) - Could use `governor` crate
- #89 (Prometheus) - Use `prometheus` crate
- #77 (mDNS) - Use `mdns-sd` crate
- #54 (Embeddings) - Use `fastembed-rs` crate

### Potential Partnerships
- #58 (MCP) - Anthropic's protocol, collaborate
- #72 (OAuth 2.1) - Complex, consider `oauth2` crate
- #94 (Multi-OAuth) - Many providers, consider existing SDKs

### Skip or Defer
- #91 (ClamAV) - Niche use case, defer to later
- #60 (Meeting Intel) - Complex pipeline, not core
- #79/#80 (Feishu/LINE) - Regional, defer unless demand

---

## Recommended Staffing Model

### Solo Developer Timeline
- **Focus**: Sequential implementation, prioritize P0 and critical P1
- **Timeline**: 88-125 weeks (~2 years)
- **Risk**: Feature creep, burnout, slow delivery
- **Mitigation**: Ruthlessly prioritize P0, defer P3

### 2-Engineer Team
- **Track A**: Security + Agent Features (Engineer 1)
- **Track B**: Reliability + Infrastructure (Engineer 2)
- **Timeline**: 44-63 weeks (~12-15 months)
- **Risk**: Context switching, integration issues
- **Mitigation**: Weekly sync, clear interfaces

### 3-Engineer Team (Recommended)
- **Track A**: Security (DevSecOps)
- **Track B**: Agent Features (ML Engineer)
- **Track C**: Infrastructure (Backend Engineer)
- **Timeline**: 30-42 weeks (~8-10 months)
- **Risk**: Communication overhead
- **Mitigation**: Shared backlog, daily standups

### 4-Engineer Team (Optimal)
- **Track A**: Security (DevSecOps)
- **Track B**: Reliability (Backend Engineer)
- **Track C**: Agent Features (ML Engineer)
- **Track D**: Developer Experience (Full-stack)
- **Timeline**: 22-32 weeks (~5-8 months)
- **Risk**: Coordination complexity
- **Mitigation**: Project manager, weekly demos

---

## Success Criteria by Phase

### Phase 1 Complete (8 weeks)
- [ ] All credentials use `Secret<T>` wrapper
- [ ] Retry engine handles 99.9% of transient failures
- [ ] Safety layer blocks 95%+ of known attack patterns
- [ ] Config validation catches typos with suggestions
- [ ] Personality files customize agent behavior

### Phase 2 Complete (14 weeks)
- [ ] Failover works across 3+ providers
- [ ] Gateway runs as systemd/launchd service
- [ ] Rate limiting prevents DoS attacks
- [ ] Message delivery succeeds 99.9% of time

### Phase 3 Complete (22 weeks)
- [ ] Conversations exceed 100K tokens via compaction
- [ ] Memory reflector extracts facts with 85%+ precision
- [ ] Sub-agents delegate tasks successfully
- [ ] Lifecycle hooks enable community plugins

---

## Conclusion

**Recommended Strategy**: Start with P0 Quick Wins (#86, #81, #52, #83, #84, #85, #70, #63, #73, #51) over 10-14 weeks to establish:
1. Secure credential handling
2. Reliable external API calls
3. Unified security defenses
4. Excellent configuration UX
5. Easy agent customization
6. Production-ready gateway

This foundation enables rapid iteration on P1-P3 features with lower integration risk.

**Key Success Factor**: Leverage the 13 features with 3+ ecosystem references (30% of roadmap) as low-risk implementations to build momentum before tackling novel IronClaw-inspired features.

---

**Full Details**: See `/mnt/developer/git/aecs4u.it/RustyClaw/DEVELOPMENT_ROADMAP.md`
**Quick Reference**: See `/mnt/developer/git/aecs4u.it/RustyClaw/ROADMAP_SUMMARY.md`
