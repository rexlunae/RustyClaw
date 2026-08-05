//! Download panel client-frame handlers (list / cancel / clear).
//!
//! Transfers started by `web_fetch` are not chat events. They outlive the
//! turn that started them, they have no reply to stream, and their progress
//! is not something the model should be re-reading — so they get their own
//! panel and their own frames rather than a place in the transcript.
//!
//! Every handler here is scoped to one connection. The registry is
//! process-global, and ids are process-wide, so without that scoping a client
//! that guessed an id could cancel another agent's download or learn where it
//! was writing.

use anyhow::Result;
use tracing::debug;

use rustyclaw_core::downloads::download_manager;
use rustyclaw_core::gateway::protocol::server::send_frame;
use rustyclaw_core::gateway::{ServerFrame, ServerFrameType, ServerPayload, protocol, transport};

/// Snapshot this agent's transfers for the wire.
///
/// The lock is taken and released here rather than held across the send: the
/// registry is a `std::sync::Mutex` shared with every running transfer's write
/// loop, and holding it across a network write would stall them all.
fn snapshot(agent: &str) -> Result<Vec<protocol::DownloadInfoDto>> {
    let downloads = download_manager()
        .lock()
        .map_err(|_| anyhow::anyhow!("the download registry is poisoned"))?
        .list_for(agent);
    Ok(downloads.into_iter().map(Into::into).collect())
}

/// Send the current transfer list to the client.
pub(crate) async fn send_downloads_update(
    writer: &mut dyn transport::TransportWriter,
    agent: &str,
) -> Result<()> {
    let downloads = snapshot(agent)?;
    debug!(count = downloads.len(), "Sending DownloadsUpdate");
    send_frame(
        writer,
        &ServerFrame {
            frame_type: ServerFrameType::DownloadsUpdate,
            payload: ServerPayload::DownloadsUpdate { downloads },
        },
    )
    .await
}

/// Handle a `DownloadCancel`.
///
/// The transfer's own write loop is what actually stops: it re-reads the
/// registry on every chunk and gives up the moment the status is terminal.
/// Nothing here touches the file — a partial download is left on disk where
/// the user can find or delete it, rather than being removed on their behalf.
pub(crate) async fn handle_download_cancel(
    writer: &mut dyn transport::TransportWriter,
    agent: &str,
    id: &str,
) -> Result<()> {
    let cancelled = download_manager()
        .lock()
        .map_err(|_| anyhow::anyhow!("the download registry is poisoned"))?
        .cancel(id, agent);
    match cancelled {
        Some(download) => {
            // Announced so every watcher agrees on the ending — including
            // this agent's own wake channel, which is how the agent
            // learns the file it was waiting for is not coming.
            rustyclaw_core::downloads::announce(download);
            // The announcement drives the panel update, so nothing is sent
            // here: sending one too would race it and could show the older
            // of the two states.
            Ok(())
        }
        None => {
            // Unknown id, already finished, or another agent's. Saying
            // which would confirm the existence of a transfer this client was
            // never shown, so it gets one answer for all three — and a fresh
            // list, which is the honest correction if its panel was stale.
            debug!(
                id,
                "Ignoring a cancel for a transfer this agent cannot stop"
            );
            send_downloads_update(writer, agent).await
        }
    }
}

/// Handle a `DownloadsClearFinished`: forget this agent's finished transfers.
/// Running ones stay, here and for every other agent.
pub(crate) async fn handle_downloads_clear_finished(
    writer: &mut dyn transport::TransportWriter,
    agent: &str,
) -> Result<()> {
    let cleared = download_manager()
        .lock()
        .map_err(|_| anyhow::anyhow!("the download registry is poisoned"))?
        .clear_finished_for(agent);
    debug!(cleared, "Cleared finished transfers");
    // Nothing announces a removal — the broadcast carries changes to
    // transfers, and these no longer exist — so the update is sent directly.
    send_downloads_update(writer, agent).await
}
