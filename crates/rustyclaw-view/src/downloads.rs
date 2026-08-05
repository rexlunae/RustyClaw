//! Component data for the downloads panel.
//!
//! A transfer started by `web_fetch` is deliberately not a chat event: it
//! outlives the turn that started it, its progress is not something the model
//! should be re-reading, and the user wants to watch it without scrolling
//! through a conversation. This is what both clients render instead.

/// Display data for one transfer.
#[derive(Clone, Debug, PartialEq)]
pub struct DownloadData {
    pub id: String,
    pub url: String,
    /// Absolute, so the user can actually find the file.
    pub dest: String,
    pub total_bytes: Option<u64>,
    pub received_bytes: u64,
    /// One of `running`, `complete`, `failed`, `cancelled` — or something a
    /// newer gateway added, which is shown as-is rather than hidden.
    pub status: String,
    pub error: Option<String>,
    pub started_ms: u64,
    pub finished_ms: Option<u64>,
}

impl From<rustyclaw_core::gateway::protocol::frames::DownloadInfoDto> for DownloadData {
    fn from(dto: rustyclaw_core::gateway::protocol::frames::DownloadInfoDto) -> Self {
        Self {
            id: dto.id,
            url: dto.url,
            dest: dto.dest.display().to_string(),
            total_bytes: dto.total_bytes,
            received_bytes: dto.received_bytes,
            status: dto.status,
            error: dto.error,
            started_ms: dto.started_ms,
            finished_ms: dto.finished_ms,
        }
    }
}

impl DownloadData {
    /// Whether the transfer is still going, and so can be cancelled.
    ///
    /// An allow-list rather than a deny-list of endings: a status this client
    /// is too old to recognise reads as *not* running. The other way round, a
    /// newer gateway's status would be offered a Cancel button this build has
    /// no idea whether the gateway will honour — and a button that silently
    /// does nothing is worse than an absent one. The transfer is still shown
    /// either way; see [`Self::status_label`].
    pub fn is_running(&self) -> bool {
        self.status == "running"
    }

    /// Progress from 0.0 to 1.0, when the server declared a length.
    ///
    /// `None` for a chunked response. The panel then shows an indeterminate
    /// bar: a percentage invented from an unknown total is worse than none,
    /// because it looks like information.
    pub fn fraction(&self) -> Option<f32> {
        match self.total_bytes {
            Some(total) if total > 0 => Some((self.received_bytes as f32 / total as f32).min(1.0)),
            _ => None,
        }
    }

    /// Progress as a whole percentage, when there is one.
    pub fn percent(&self) -> Option<u8> {
        self.fraction().map(|f| (f * 100.0).round() as u8)
    }

    /// The file's name, for a panel with no room for the whole path.
    pub fn file_name(&self) -> &str {
        self.dest
            .rsplit(['/', '\\'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.dest)
    }

    /// "1.2 MB of 4.0 MB", or just what has arrived when the total is unknown.
    pub fn size_display(&self) -> String {
        let received = rustyclaw_core::downloads::human_bytes(self.received_bytes);
        match self.total_bytes {
            Some(total) => format!(
                "{received} of {}",
                rustyclaw_core::downloads::human_bytes(total)
            ),
            None => received,
        }
    }

    /// A short word for the status badge.
    pub fn status_label(&self) -> &str {
        match self.status.as_str() {
            "running" => "Downloading",
            "complete" => "Complete",
            "failed" => "Failed",
            "cancelled" => "Cancelled",
            other => other,
        }
    }

    /// CSS-like class for the status badge, matching the other panels.
    pub fn status_class(&self) -> &'static str {
        match self.status.as_str() {
            "running" => "is-info",
            "complete" => "is-success",
            "failed" => "is-danger",
            "cancelled" => "is-warning",
            _ => "is-light",
        }
    }

    /// How long it ran, once it has stopped.
    pub fn duration_display(&self) -> Option<String> {
        let elapsed_ms = self.finished_ms?.saturating_sub(self.started_ms);
        let secs = elapsed_ms / 1000;
        Some(match secs {
            0 => format!("{elapsed_ms}ms"),
            s if s < 60 => format!("{s}s"),
            s => format!("{}m {}s", s / 60, s % 60),
        })
    }
}

/// Data for the downloads panel.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DownloadsData {
    /// Newest first, as the gateway sends them.
    pub downloads: Vec<DownloadData>,
}

impl DownloadsData {
    /// Whether there is nothing to show — what the panel's empty state keys on.
    pub fn is_empty(&self) -> bool {
        self.downloads.is_empty()
    }

    /// How many are still going. What a badge on the panel's button shows —
    /// a count of everything ever downloaded is not news.
    pub fn active_count(&self) -> usize {
        self.downloads.iter().filter(|d| d.is_running()).count()
    }

    /// Whether anything can be cleared, so the button can be disabled rather
    /// than doing nothing.
    pub fn has_finished(&self) -> bool {
        self.downloads.iter().any(|d| !d.is_running())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn download(status: &str, received: u64, total: Option<u64>) -> DownloadData {
        DownloadData {
            id: "dl_1".into(),
            url: "https://example.test/big.iso".into(),
            dest: "/home/u/dl/big.iso".into(),
            total_bytes: total,
            received_bytes: received,
            status: status.into(),
            error: None,
            started_ms: 1_000,
            finished_ms: None,
        }
    }

    /// A status this client has never heard of must not be offered a Cancel
    /// button — whatever it means, this build cannot act on it, and a button
    /// that does nothing is worse than no button.
    #[test]
    fn an_unrecognised_status_is_not_treated_as_still_running() {
        assert!(download("running", 0, None).is_running());
        assert!(!download("complete", 0, None).is_running());
        assert!(
            !download("quarantined", 0, None).is_running(),
            "a newer gateway's status must not look cancellable"
        );
    }

    /// ...but it is still shown, rather than being dropped from the panel.
    #[test]
    fn an_unrecognised_status_is_still_displayed() {
        assert_eq!(
            download("quarantined", 0, None).status_label(),
            "quarantined"
        );
    }

    #[test]
    fn a_chunked_response_has_no_percentage() {
        assert_eq!(download("running", 5_000, None).percent(), None);
        assert_eq!(download("running", 512, Some(1024)).percent(), Some(50));
    }

    /// A server that under-reports its length must not produce 130%.
    #[test]
    fn progress_past_the_declared_size_is_clamped() {
        assert_eq!(download("running", 1300, Some(1000)).percent(), Some(100));
    }

    #[test]
    fn the_panel_counts_only_transfers_still_going() {
        let data = DownloadsData {
            downloads: vec![
                download("running", 1, None),
                download("complete", 1, None),
                download("failed", 1, None),
            ],
        };
        assert_eq!(data.active_count(), 1);
        assert!(data.has_finished());
    }

    #[test]
    fn a_size_reads_as_a_size() {
        assert_eq!(
            download("running", 1024 * 1024, Some(4 * 1024 * 1024)).size_display(),
            "1.0 MB of 4.0 MB"
        );
        assert_eq!(download("running", 2048, None).size_display(), "2.0 KB");
    }

    #[test]
    fn the_file_name_is_the_last_path_segment() {
        assert_eq!(download("running", 0, None).file_name(), "big.iso");
    }
}
