//! Component data for the cron/scheduled-jobs management panel.

use crate::tone::Tone;

/// Display data for a single cron job.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CronJobData {
    pub id: String,
    pub name: String,
    pub expr: String,
    pub payload: String,
    pub paused: bool,
    pub next_run: Option<String>,
    pub last_run: Option<String>,
    pub last_status: Option<String>,
    pub run_count: u64,
}

impl CronJobData {
    /// Convert from the protocol DTO.
    pub fn from_dto(dto: &rustyclaw_core::gateway::protocol::frames::CronJobDto) -> Self {
        Self {
            id: dto.id.clone(),
            name: dto.name.clone(),
            expr: dto.expr.clone(),
            payload: dto.payload.clone(),
            paused: dto.paused,
            next_run: dto.next_run.clone(),
            last_run: dto.last_run.clone(),
            last_status: dto.last_status.clone(),
            run_count: dto.run_count,
        }
    }

    /// Status badge tone.
    pub fn status_tone(&self) -> Tone {
        if self.paused {
            return Tone::Neutral;
        }
        match self.last_status.as_deref() {
            Some("ok") | Some("success") => Tone::Success,
            Some("error") | Some("failed") => Tone::Danger,
            Some("running") => Tone::Info,
            _ => Tone::Warning,
        }
    }

    /// Status display label.
    pub fn status_label(&self) -> &str {
        if self.paused {
            return "Paused";
        }
        match self.last_status.as_deref() {
            Some(s) => s,
            None => "Pending",
        }
    }
}

/// Full state for the cron management panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CronPanelData {
    pub jobs: Vec<CronJobData>,
    pub selected: Option<usize>,
    pub status: Option<String>,
}

impl CronPanelData {
    pub fn active_count(&self) -> usize {
        self.jobs.iter().filter(|j| !j.paused).count()
    }

    pub fn total_count(&self) -> usize {
        self.jobs.len()
    }

    pub fn selected_job(&self) -> Option<&CronJobData> {
        self.selected.and_then(|i| self.jobs.get(i))
    }

    pub fn select_next(&mut self) {
        let max = self.jobs.len().saturating_sub(1);
        let cur = self.selected.unwrap_or(0);
        self.selected = Some(if cur >= max { 0 } else { cur + 1 });
    }

    pub fn select_prev(&mut self) {
        let max = self.jobs.len().saturating_sub(1);
        let cur = self.selected.unwrap_or(0);
        self.selected = Some(if cur == 0 { max } else { cur - 1 });
    }
}
