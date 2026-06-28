//! Component data for the memory browser panel.

/// Display data for a single memory entry.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MemoryEntryData {
    pub id: String,
    pub content: String,
    pub category: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub score: Option<f64>,
}

impl MemoryEntryData {
    /// Convert from the protocol DTO.
    pub fn from_dto(dto: &rustyclaw_core::gateway::protocol::frames::MemoryEntryDto) -> Self {
        Self {
            id: dto.id.clone(),
            content: dto.content.clone(),
            category: dto.category.clone(),
            created_at: dto.created_at.clone(),
            updated_at: dto.updated_at.clone(),
            score: dto.score,
        }
    }

    /// Category badge label (or "General" fallback).
    pub fn category_label(&self) -> &str {
        self.category.as_deref().unwrap_or("General")
    }

    /// Truncated content preview for list views.
    pub fn preview(&self, max_chars: usize) -> &str {
        if self.content.len() <= max_chars {
            &self.content
        } else {
            &self.content[..max_chars]
        }
    }

    /// Relevance score as percentage string (for search results).
    pub fn score_display(&self) -> Option<String> {
        self.score.map(|s| format!("{:.0}%", s * 100.0))
    }
}

/// Display data for a history entry.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct HistoryEntryData {
    pub timestamp: String,
    pub role: String,
    pub content: String,
    pub thread_id: Option<u64>,
}

impl HistoryEntryData {
    pub fn from_dto(dto: &rustyclaw_core::gateway::protocol::frames::HistoryEntryDto) -> Self {
        Self {
            timestamp: dto.timestamp.clone(),
            role: dto.role.clone(),
            content: dto.content.clone(),
            thread_id: dto.thread_id,
        }
    }

    /// Role icon for display.
    pub fn role_icon(&self) -> &'static str {
        match self.role.as_str() {
            "user" => "🧑",
            "assistant" => "🦞",
            "system" => "⚙",
            _ => "ℹ️",
        }
    }
}

/// Full state for the memory browser panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MemoryPanelData {
    pub entries: Vec<MemoryEntryData>,
    pub history: Vec<HistoryEntryData>,
    pub selected: Option<usize>,
    pub search_query: String,
    pub editing: bool,
    pub edit_content: String,
    pub status: Option<String>,
}

impl MemoryPanelData {
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    pub fn selected_entry(&self) -> Option<&MemoryEntryData> {
        self.selected.and_then(|i| self.entries.get(i))
    }

    pub fn select_next(&mut self) {
        let max = self.entries.len().saturating_sub(1);
        let cur = self.selected.unwrap_or(0);
        self.selected = Some(if cur >= max { 0 } else { cur + 1 });
    }

    pub fn select_prev(&mut self) {
        let max = self.entries.len().saturating_sub(1);
        let cur = self.selected.unwrap_or(0);
        self.selected = Some(if cur == 0 { max } else { cur - 1 });
    }
}
