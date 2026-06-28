//! Component data for the token/usage analytics dashboard.

/// Aggregate usage totals for display.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct UsageTotalsData {
    pub total_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_latency_ms: u64,
    pub period: String,
}

impl UsageTotalsData {
    pub fn from_dto(dto: &rustyclaw_core::gateway::protocol::frames::UsageTotalsDto) -> Self {
        Self {
            total_requests: dto.total_requests,
            total_input_tokens: dto.total_input_tokens,
            total_output_tokens: dto.total_output_tokens,
            total_latency_ms: dto.total_latency_ms,
            period: dto.period.clone(),
        }
    }

    /// Total tokens (input + output).
    pub fn total_tokens(&self) -> u64 {
        self.total_input_tokens + self.total_output_tokens
    }

    /// Average latency per request in milliseconds.
    pub fn avg_latency_ms(&self) -> u64 {
        self.total_latency_ms
            .checked_div(self.total_requests)
            .unwrap_or(0)
    }

    /// Human-readable token count (e.g. "1.2M", "450K").
    pub fn tokens_display(tokens: u64) -> String {
        if tokens >= 1_000_000 {
            format!("{:.1}M", tokens as f64 / 1_000_000.0)
        } else if tokens >= 1_000 {
            format!("{:.1}K", tokens as f64 / 1_000.0)
        } else {
            tokens.to_string()
        }
    }
}

/// Per-model usage for display.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ModelUsageData {
    pub provider: String,
    pub model: String,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub avg_latency_ms: u64,
}

impl ModelUsageData {
    pub fn from_dto(dto: &rustyclaw_core::gateway::protocol::frames::ModelUsageDto) -> Self {
        Self {
            provider: dto.provider.clone(),
            model: dto.model.clone(),
            requests: dto.requests,
            input_tokens: dto.input_tokens,
            output_tokens: dto.output_tokens,
            avg_latency_ms: dto.avg_latency_ms,
        }
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// Per-session usage for display.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SessionUsageData {
    pub session_id: String,
    pub thread_label: Option<String>,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl SessionUsageData {
    pub fn from_dto(dto: &rustyclaw_core::gateway::protocol::frames::SessionUsageDto) -> Self {
        Self {
            session_id: dto.session_id.clone(),
            thread_label: dto.thread_label.clone(),
            requests: dto.requests,
            input_tokens: dto.input_tokens,
            output_tokens: dto.output_tokens,
        }
    }

    pub fn display_label(&self) -> &str {
        self.thread_label.as_deref().unwrap_or(&self.session_id)
    }
}

/// Full state for the analytics panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AnalyticsPanelData {
    pub totals: UsageTotalsData,
    pub per_model: Vec<ModelUsageData>,
    pub per_session: Vec<SessionUsageData>,
    pub period: String,
    pub status: Option<String>,
}
