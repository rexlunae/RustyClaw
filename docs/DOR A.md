# DORA Metrics

**Document Status**: Implementation Guide
**Last Updated**: 2026-02-16
**Tracking Issues**: #89, #90, #92

---

## Overview

RustyClaw implements [DORA (DevOps Research and Assessment) metrics](https://dora.dev/) to measure and improve software delivery performance. These metrics provide objective data about our development velocity, stability, and operational excellence.

## The Four Key Metrics

### 1. Deployment Frequency (DF)

**Definition**: How often code is successfully deployed to production.

**Measurement**: Number of deployments per day (averaged over 7 days).

**Data Source**: Git tags matching `v*` pattern (e.g., `v0.1.0`, `v1.2.3`).

**Performance Levels**:
- **Elite** ✅: Multiple deployments per day (>1/day)
- **High** 🟢: Daily to weekly (0.14-1/day)
- **Medium** 🟡: Weekly to monthly (0.03-0.14/day)
- **Low** 🔴: Less than monthly (<0.03/day)

**Why It Matters**: Higher deployment frequency correlates with smaller batch sizes, faster feedback loops, and reduced risk per deployment.

### 2. Lead Time for Changes (LT)

**Definition**: Time from code commit to running in production.

**Measurement**: Average time (in hours) between commit timestamp and release tag creation (last 5 releases).

**Data Source**: Git commit history and tag timestamps.

**Performance Levels**:
- **Elite** ✅: Less than 1 day (<24h)
- **High** 🟢: 1-7 days (24-168h)
- **Medium** 🟡: 1-4 weeks (168-720h)
- **Low** 🔴: More than 4 weeks (>720h)

**Why It Matters**: Shorter lead times enable faster response to user feedback and market changes. Long lead times indicate bottlenecks in your development process.

### 3. Change Failure Rate (CFR)

**Definition**: Percentage of deployments that cause failures requiring remediation.

**Measurement**: (Hotfixes + Rollbacks + Reverts) / Total Deployments × 100% (last 30 days).

**Data Source**: Git commits with keywords `hotfix`, `rollback`, or `revert` in the message.

**Performance Levels**:
- **Elite/High** ✅: 0-15%
- **Medium** 🟡: 15-30%
- **Low** 🔴: >30%

**Why It Matters**: Low failure rates indicate high-quality releases and effective testing. High rates suggest insufficient quality gates or overly aggressive release pace.

### 4. Mean Time to Recovery (MTTR)

**Definition**: Average time to restore service after a production incident.

**Measurement**: Average time (in hours) from incident creation to resolution (last 20 incidents).

**Data Source**: GitHub issues with `incident` label (time between opened and closed).

**Performance Levels**:
- **Elite** ✅: Less than 1 hour (<1h)
- **High** 🟢: Less than 1 day (<24h)
- **Medium** 🟡: 1-7 days (24-168h)
- **Low** 🔴: More than 7 days (>168h)

**Why It Matters**: Fast recovery minimizes user impact and demonstrates operational maturity. MTTR improvements often come from better monitoring, automation, and runbooks.

---

## Implementation

### Automated Collection

DORA metrics are automatically calculated daily via GitHub Actions:

**Workflow**: [`.github/workflows/dora-metrics.yml`](../.github/workflows/dora-metrics.yml)

**Triggers**:
- **Scheduled**: Daily at 00:00 UTC
- **Manual**: Via `workflow_dispatch`
- **Automatic**: On release publish or tag push (`v*`)

**Outputs**:
1. **Workflow Summary**: Detailed report visible in Actions tab
2. **Badge JSON Files**: Stored in `.dora/` directory for shields.io integration
3. **Git Commit**: Updates badge data (auto-committed to `main` branch)

### Viewing Metrics

#### Option 1: GitHub Actions Workflow Summary

1. Navigate to [Actions → DORA Metrics](https://github.com/aecs4u/RustyClaw/actions/workflows/dora-metrics.yml)
2. Click on the most recent workflow run
3. Expand the **"Calculate DORA Metrics"** job
4. Scroll to **"Generate DORA Report"** step summary

**Example Output**:
```
📊 DORA Metrics Report

Generated: 2026-02-16 14:32 UTC
Repository: aecs4u/RustyClaw

Four Key Metrics

| Metric                    | Value          | Performance Level |
|---------------------------|----------------|-------------------|
| Deployment Frequency      | 0.14/day (1/7d)| High 🟢           |
| Lead Time for Changes     | 18.5h          | Elite ✅          |
| Change Failure Rate       | 5.0% (1/20)    | Elite/High ✅     |
| Mean Time to Recovery     | 12.3h (8 incidents) | High 🟢      |
```

#### Option 2: Shields.io Badges (README)

Badges display real-time metrics from the `.dora/*.json` files:

[![Deployment Frequency](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/aecs4u/RustyClaw/main/.dora/deployment-frequency.json)](https://github.com/aecs4u/RustyClaw/actions/workflows/dora-metrics.yml)
[![Lead Time](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/aecs4u/RustyClaw/main/.dora/lead-time.json)](https://github.com/aecs4u/RustyClaw/actions/workflows/dora-metrics.yml)

**Badge Colors**:
- 🟢 **Green** (brightgreen/green): Elite or High performance
- 🟡 **Yellow**: Medium performance
- 🔴 **Red**: Low performance, needs improvement

#### Option 3: Manual Calculation Script

For ad-hoc analysis, run the calculation script locally:

```bash
# Clone the repository
git clone https://github.com/aecs4u/RustyClaw.git
cd RustyClaw

# Run the calculation script (requires GitHub CLI for MTTR)
bash .github/scripts/calculate-dora.sh
```

---

## Tracking Production Incidents

To accurately measure MTTR, production incidents must be tracked as GitHub issues with the `incident` label.

### Creating an Incident

1. **Open a new issue** with template:
   ```markdown
   **Incident**: [Brief description]

   **Severity**: Critical | High | Medium | Low
   **Detected**: [Timestamp when first detected]
   **Impact**: [User-facing impact description]
   **Root Cause** (if known): [Technical explanation]

   ---

   **Investigation Timeline**:
   - [ ] Incident detected and triaged
   - [ ] Root cause identified
   - [ ] Fix implemented
   - [ ] Fix deployed
   - [ ] Monitoring confirms resolution
   - [ ] Postmortem completed
   ```

2. **Add the `incident` label**

3. **Assign severity label**: `severity:critical`, `severity:high`, `severity:medium`, or `severity:low`

4. **Close the issue** when service is fully restored

### Automated Incident Tracking

The incident tracker workflow automatically responds to incident-labeled issues:

**Workflow**: [`.github/workflows/incident-tracker.yml`](../.github/workflows/incident-tracker.yml)

**Triggers**:
- Issue opened with `incident` label
- Issue closed with `incident` label

**Actions**:
- Logs incident opened event
- Calculates MTTR when incident is closed
- Posts recovery time as a comment on the issue

**Example Comment**:
```
✅ Incident Resolved

- Time to Recovery: 4.2 hours
- Status: Closed
- Severity: High
```

---

## Improving Your Metrics

### Deployment Frequency

**Low → Medium**:
- ✅ Automate the release process (GitHub Actions already in place)
- ✅ Reduce manual approval gates
- ✅ Implement feature flags for safe deployments

**Medium → High**:
- ✅ Move to continuous deployment (CD)
- ✅ Deploy on every merge to `main`
- ✅ Use blue-green or canary deployments

**High → Elite**:
- ✅ Deploy multiple times per day
- ✅ Implement progressive delivery
- ✅ Use automated rollback on failure detection

### Lead Time for Changes

**Reduce from weeks to days**:
- ✅ Parallelize CI/CD pipeline stages
- ✅ Reduce PR review time (set SLAs, automate review requests)
- ✅ Break large features into smaller PRs

**Reduce from days to hours**:
- ✅ Implement automated testing (unit, integration, e2e)
- ✅ Use trunk-based development (short-lived branches)
- ✅ Automate release notes and changelog generation

**Reduce to <1 day (Elite)**:
- ✅ Fully automated deployment pipeline
- ✅ Instant PR feedback (<5 minutes)
- ✅ Remove manual approval steps

### Change Failure Rate

**Reduce from >30% to <30%**:
- ✅ Add comprehensive test coverage (RustyClaw has 330+ tests)
- ✅ Implement pre-commit hooks (linting, formatting)
- ✅ Use staging environment for validation

**Reduce from <30% to <15% (Elite)**:
- ✅ Add integration tests with realistic data
- ✅ Implement canary deployments (gradual rollout)
- ✅ Use feature flags to decouple deploy from release

**Maintain <5% (Best-in-class)**:
- ✅ Property-based testing and fuzzing
- ✅ Chaos engineering experiments
- ✅ Automated rollback on anomaly detection

### Mean Time to Recovery

**Reduce from >7 days to <7 days**:
- ✅ Implement monitoring and alerting
- ✅ Create incident response runbooks
- ✅ Establish on-call rotation

**Reduce from <7 days to <1 day (High)**:
- ✅ Automated deployment rollback
- ✅ Feature flags for instant disable
- ✅ Comprehensive observability (logs, metrics, traces)

**Reduce to <1 hour (Elite)**:
- ✅ Fully automated detection and recovery
- ✅ Self-healing systems
- ✅ Blue-green deployments with instant failover

---

## Integration with Existing Systems

### Prometheus Metrics

RustyClaw's existing `/metrics` endpoint (issue #89) can be extended with DORA-specific metrics:

```rust
// Add to src/metrics.rs

lazy_static! {
    pub static ref DEPLOYMENTS_TOTAL: CounterVec = register_counter_vec!(
        "rustyclaw_deployments_total",
        "Total number of deployments",
        &["environment", "result"]
    ).unwrap();

    pub static ref LEAD_TIME_SECONDS: HistogramVec = register_histogram_vec!(
        "rustyclaw_lead_time_seconds",
        "Lead time from commit to deployment",
        &["environment"],
        vec![60.0, 300.0, 900.0, 3600.0, 14400.0, 86400.0]
    ).unwrap();

    pub static ref INCIDENTS_TOTAL: CounterVec = register_counter_vec!(
        "rustyclaw_incidents_total",
        "Total production incidents",
        &["severity", "status"]
    ).unwrap();

    pub static ref MTTR_SECONDS: HistogramVec = register_histogram_vec!(
        "rustyclaw_mttr_seconds",
        "Mean time to recovery",
        &["severity"],
        vec![300.0, 1800.0, 3600.0, 14400.0, 86400.0]
    ).unwrap();
}
```

### Grafana Dashboard

A Grafana dashboard configuration is available at [`docs/grafana/dora-dashboard.json`](./grafana/dora-dashboard.json) for visualizing DORA metrics alongside other operational metrics.

**Datasource**: Prometheus (querying RustyClaw's `/metrics` endpoint)

**Panels**:
- Deployment Frequency (7-day rolling average)
- Lead Time Distribution (P50, P95, P99)
- Change Failure Rate (30-day rolling)
- MTTR Trend (by severity)

---

## Benchmarking and Goals

### Current Status

Run the DORA metrics workflow to see RustyClaw's current performance:

```bash
gh workflow run dora-metrics.yml --repo aecs4u/RustyClaw
gh run list --workflow=dora-metrics.yml --limit 1
```

### Target Metrics (2026 Goals)

| Metric | Current | Q2 2026 Target | Q4 2026 Target |
|--------|---------|----------------|----------------|
| Deployment Frequency | TBD | 0.5/day (High) | 1+/day (Elite) |
| Lead Time | TBD | <24h (Elite) | <12h (Elite) |
| Change Failure Rate | TBD | <15% (Elite/High) | <10% (Elite) |
| MTTR | TBD | <24h (High) | <12h (High) |

### Industry Benchmarks (DORA 2023)

Based on the [2023 Accelerate State of DevOps Report](https://dora.dev/research/2023/dora-report/):

- **Elite performers**: Top 7% of organizations
- **High performers**: 48% of organizations
- **Medium performers**: 35% of organizations
- **Low performers**: 10% of organizations

**Key Finding**: Elite performers deploy 973x more frequently and recover from failures 6,570x faster than low performers.

---

## FAQ

### Why are my metrics showing as 0 or N/A?

**Deployment Frequency = 0**: No release tags (`v*`) in the last 7 days. Create a release to start tracking.

**Lead Time = 0**: Less than 5 release tags exist. Metrics become more accurate with more data.

**Change Failure Rate = 0**: No commits with "hotfix"/"rollback"/"revert" in the last 30 days (good sign!) or no releases to calculate against.

**MTTR = 0**: No issues with the `incident` label have been closed. Create and track incidents as they occur.

### How often are badges updated?

Badges update:
1. **Daily** at 00:00 UTC (scheduled workflow)
2. **On release** when a `v*` tag is pushed
3. **On demand** via manual workflow trigger

Shields.io may cache badge images for up to 5 minutes.

### Can I exclude certain releases from metrics?

Yes. Releases with `-rc`, `-alpha`, `-beta`, `-pre` suffixes are automatically excluded from production deployment frequency calculations. Example: `v1.2.0-rc1` won't count as a production deployment.

### How do I track incidents for services deployed outside GitHub?

For external incident tracking (PagerDuty, Datadog, etc.), you can:

1. **Option 1**: Manually create GitHub issues with the `incident` label when external incidents occur
2. **Option 2**: Use a webhook to auto-create GitHub issues from external systems
3. **Option 3**: Extend the workflow to query external APIs (requires custom integration)

### What if my MTTR is artificially low?

If incidents are being closed prematurely (before actual resolution), this will skew MTTR. Best practices:

- Only close the incident when service is **fully restored** and monitoring confirms stability
- Add a comment with resolution details before closing
- For multi-day incidents, add progress updates as comments
- Consider using `incident:investigating`, `incident:fixing`, `incident:resolved` labels for granular tracking

---

## Related Documentation

- **[ROADMAP_SUMMARY.md](../ROADMAP_SUMMARY.md)** — Issue #89 (Prometheus metrics), #90 (Per-tool cost tracking), #92 (Activity status)
- **[CONTRIBUTING.md](./CONTRIBUTING.md)** — How to contribute to improving DORA metrics
- **[CI/CD Workflows](../.github/workflows/)** — Automation pipelines
- **[dora.dev](https://dora.dev/)** — Official DORA research and resources
- **[Accelerate State of DevOps Report](https://dora.dev/research/)** — Annual industry benchmarks

---

## References

1. Forsgren, N., Humble, J., & Kim, G. (2018). *Accelerate: The Science of Lean Software and DevOps*. IT Revolution Press.
2. [DORA Metrics: How to Measure Software Delivery Performance](https://dora.dev/guides/dora-metrics-four-keys/)
3. [2023 Accelerate State of DevOps Report](https://dora.dev/research/2023/dora-report/)
4. [Google Cloud: Four Keys Project](https://github.com/dora-team/fourkeys)
5. [Measuring DevOps Metrics: A Practical Guide](https://cloud.google.com/blog/products/devops-sre/using-the-four-keys-to-measure-your-devops-performance)

---

**Last Updated**: 2026-02-16
**Maintainers**: [@aecs4u](https://github.com/aecs4u)
**Questions?**: [Open a discussion](https://github.com/aecs4u/RustyClaw/discussions)
