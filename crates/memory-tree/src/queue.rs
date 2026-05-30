//! Durable, lease-based job queue.
//!
//! Workers reserve a job for a fixed lease window; expired leases are eligible
//! again so a crashed worker doesn't strand work. Dedupe keys prevent the same
//! conceptual job from being enqueued twice while incomplete.

use crate::error::Result;
use crate::store::Store;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Deep extraction + entity scoring for a chunk.
    ExtractChunk,
    /// Add an admitted leaf to a source's L0 buffer.
    AppendBuffer,
    /// Compress an L0 buffer into an L1 summary.
    Seal,
    /// Route a leaf into per-entity topic trees.
    TopicRoute,
    /// Build the global daily digest.
    DigestDaily,
    /// Force-seal stale buffers.
    FlushStale,
}

impl JobKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExtractChunk => "extract_chunk",
            Self::AppendBuffer => "append_buffer",
            Self::Seal => "seal",
            Self::TopicRoute => "topic_route",
            Self::DigestDaily => "digest_daily",
            Self::FlushStale => "flush_stale",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "extract_chunk" => Self::ExtractChunk,
            "append_buffer" => Self::AppendBuffer,
            "seal" => Self::Seal,
            "topic_route" => Self::TopicRoute,
            "digest_daily" => Self::DigestDaily,
            "flush_stale" => Self::FlushStale,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: i64,
    pub kind: JobKind,
    pub payload: serde_json::Value,
    pub attempts: u32,
    pub scheduled_at: DateTime<Utc>,
}

pub struct Queue {
    store: Arc<Store>,
    /// How long a worker holds a job before its lease expires. Default 60s.
    pub lease: Duration,
}

impl Queue {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            lease: Duration::seconds(60),
        }
    }

    /// Enqueue a job. Returns `Ok(Some(id))` on insert, `Ok(None)` if a
    /// matching `dedupe_key` is already in flight.
    pub fn enqueue(
        &self,
        kind: JobKind,
        payload: serde_json::Value,
        dedupe_key: Option<&str>,
    ) -> Result<Option<i64>> {
        let payload_str = serde_json::to_string(&payload)?;
        let now = Utc::now().to_rfc3339();
        self.store.with_conn(|c| {
            let result = c.execute(
                "INSERT INTO jobs (kind, payload, dedupe_key, scheduled_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![kind.as_str(), &payload_str, dedupe_key, &now],
            );
            match result {
                Ok(_) => Ok(Some(c.last_insert_rowid())),
                Err(rusqlite::Error::SqliteFailure(e, _))
                    if e.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    Ok(None)
                }
                Err(e) => Err(e.into()),
            }
        })
    }

    /// Reserve the next eligible job (FIFO by `scheduled_at`). Atomic: only
    /// one worker will see any given job until its lease expires.
    pub fn reserve(&self) -> Result<Option<Job>> {
        let now = Utc::now();
        let lease_until = now + self.lease;
        let now_s = now.to_rfc3339();
        let lease_s = lease_until.to_rfc3339();

        self.store.with_conn(|c| {
            let tx = c.transaction()?;
            let row: Option<(i64, String, String, i64, String)> = tx
                .query_row(
                    "SELECT id, kind, payload, attempts, scheduled_at
                     FROM jobs
                     WHERE completed_at IS NULL
                       AND scheduled_at <= ?1
                       AND (lease_until IS NULL OR lease_until < ?1)
                     ORDER BY scheduled_at ASC
                     LIMIT 1",
                    params![&now_s],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .optional()?;

            let Some((id, kind_s, payload_s, attempts, sched_s)) = row else {
                tx.commit()?;
                return Ok(None);
            };

            tx.execute(
                "UPDATE jobs SET lease_until = ?1, attempts = attempts + 1 WHERE id = ?2",
                params![&lease_s, id],
            )?;
            tx.commit()?;

            let kind = JobKind::parse(&kind_s).ok_or_else(|| {
                crate::error::MemoryTreeError::InvalidInput(format!("unknown job kind {}", kind_s))
            })?;
            let payload: serde_json::Value = serde_json::from_str(&payload_s)?;
            let scheduled_at = sched_s.parse::<DateTime<Utc>>().map_err(|e| {
                crate::error::MemoryTreeError::InvalidInput(format!("bad scheduled_at: {}", e))
            })?;
            Ok(Some(Job {
                id,
                kind,
                payload,
                attempts: (attempts + 1) as u32,
                scheduled_at,
            }))
        })
    }

    pub fn complete(&self, job_id: i64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.store.with_conn(|c| {
            c.execute(
                "UPDATE jobs SET completed_at = ?1, lease_until = NULL WHERE id = ?2",
                params![&now, job_id],
            )?;
            Ok(())
        })
    }

    pub fn fail(&self, job_id: i64, err: &str, retry_after: Option<Duration>) -> Result<()> {
        let now = Utc::now();
        let next = now + retry_after.unwrap_or(Duration::seconds(30));
        let next_s = next.to_rfc3339();
        self.store.with_conn(|c| {
            c.execute(
                "UPDATE jobs SET last_error = ?1, lease_until = NULL, scheduled_at = ?2 WHERE id = ?3",
                params![err, &next_s, job_id],
            )?;
            Ok(())
        })
    }

    pub fn pending_count(&self) -> Result<u64> {
        self.store.with_conn(|c| {
            let n: i64 = c.query_row(
                "SELECT COUNT(*) FROM jobs WHERE completed_at IS NULL",
                [],
                |r| r.get(0),
            )?;
            Ok(n as u64)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn q() -> Queue {
        Queue::new(Arc::new(Store::in_memory().unwrap()))
    }

    #[test]
    fn enqueue_reserve_complete() {
        let q = q();
        let id = q
            .enqueue(JobKind::Seal, json!({"source": "x"}), Some("seal:x"))
            .unwrap()
            .unwrap();
        let job = q.reserve().unwrap().unwrap();
        assert_eq!(job.id, id);
        assert_eq!(job.kind, JobKind::Seal);
        assert_eq!(job.attempts, 1);
        q.complete(job.id).unwrap();
        assert_eq!(q.pending_count().unwrap(), 0);
    }

    #[test]
    fn dedupe_key_blocks_duplicate() {
        let q = q();
        let a = q.enqueue(JobKind::Seal, json!({}), Some("seal:x")).unwrap();
        let b = q.enqueue(JobKind::Seal, json!({}), Some("seal:x")).unwrap();
        assert!(a.is_some());
        assert!(b.is_none());
    }

    #[test]
    fn fail_reschedules() {
        let q = Queue {
            lease: Duration::milliseconds(50),
            ..q()
        };
        q.enqueue(JobKind::Seal, json!({}), None).unwrap();
        let job = q.reserve().unwrap().unwrap();
        q.fail(job.id, "boom", Some(Duration::milliseconds(0)))
            .unwrap();
        // Reserve again — should be eligible after fail()'s reschedule.
        let job2 = q.reserve().unwrap().unwrap();
        assert_eq!(job2.id, job.id);
        assert_eq!(job2.attempts, 2);
    }

    #[test]
    fn expired_lease_makes_job_eligible_again() {
        let q = Queue {
            lease: Duration::milliseconds(1),
            ..q()
        };
        q.enqueue(JobKind::Seal, json!({}), None).unwrap();
        let _ = q.reserve().unwrap().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let again = q.reserve().unwrap();
        assert!(again.is_some());
    }
}
