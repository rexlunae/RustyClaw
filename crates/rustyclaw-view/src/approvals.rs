//! Component data for the approvals queue panel.

/// Display data for a pending approval.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PendingApprovalData {
    pub id: String,
    pub tool_name: String,
    pub arguments: String,
    pub requested_at: String,
    pub selected: bool,
}

impl PendingApprovalData {
    /// Convert from the protocol DTO.
    pub fn from_dto(dto: &rustyclaw_core::gateway::protocol::frames::PendingApprovalDto) -> Self {
        Self {
            id: dto.id.clone(),
            tool_name: dto.tool_name.clone(),
            arguments: dto.arguments.clone(),
            requested_at: dto.requested_at.clone(),
            selected: false,
        }
    }

    /// Truncated arguments preview.
    pub fn arguments_preview(&self, max_chars: usize) -> &str {
        if self.arguments.len() <= max_chars {
            &self.arguments
        } else {
            &self.arguments[..max_chars]
        }
    }
}

/// Full state for the approvals queue panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ApprovalsPanelData {
    pub approvals: Vec<PendingApprovalData>,
    pub cursor: Option<usize>,
    pub status: Option<String>,
}

impl ApprovalsPanelData {
    pub fn count(&self) -> usize {
        self.approvals.len()
    }

    /// Number of selected approvals (for batch actions).
    pub fn selected_count(&self) -> usize {
        self.approvals.iter().filter(|a| a.selected).count()
    }

    /// IDs of all selected approvals.
    pub fn selected_ids(&self) -> Vec<String> {
        self.approvals
            .iter()
            .filter(|a| a.selected)
            .map(|a| a.id.clone())
            .collect()
    }

    /// Toggle selection on the cursor item.
    pub fn toggle_current(&mut self) {
        if let Some(idx) = self.cursor {
            if let Some(item) = self.approvals.get_mut(idx) {
                item.selected = !item.selected;
            }
        }
    }

    /// Select all.
    pub fn select_all(&mut self) {
        for a in &mut self.approvals {
            a.selected = true;
        }
    }

    /// Deselect all.
    pub fn deselect_all(&mut self) {
        for a in &mut self.approvals {
            a.selected = false;
        }
    }

    pub fn cursor_next(&mut self) {
        let max = self.approvals.len().saturating_sub(1);
        let cur = self.cursor.unwrap_or(0);
        self.cursor = Some(if cur >= max { 0 } else { cur + 1 });
    }

    pub fn cursor_prev(&mut self) {
        let max = self.approvals.len().saturating_sub(1);
        let cur = self.cursor.unwrap_or(0);
        self.cursor = Some(if cur == 0 { max } else { cur - 1 });
    }
}
