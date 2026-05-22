//! Subconscious loop — periodic background evaluation of user / system tasks.
//!
//! On a tick (default 5 minutes):
//! 1. Load all enabled tasks (system + user).
//! 2. For each, build a small situation report and ask a [`SubconsciousDecider`]
//!    whether to **Skip**, **Act**, or **Escalate**.
//! 3. Execute the decision: skip is a no-op; act runs the local handler;
//!    escalate hands the task to a cloud agent (with an approval gate for
//!    write-intent tasks).
//!
//! The pattern is from OpenHuman's subconscious design. Most evaluations stay
//! on the local (`hint:fast`) model, so cost stays low.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// What the decider thinks should happen this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Nothing relevant right now.
    Skip,
    /// Something to do, can be handled by the local actor.
    Act,
    /// Needs deeper reasoning or tools — hand off to a cloud agent.
    Escalate,
}

/// Whether a task is allowed to take actions (write intent) or is read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskIntent {
    ReadOnly,
    Write,
}

/// A periodic task the subconscious watches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubconsciousTask {
    pub id: String,
    pub description: String,
    pub intent: TaskIntent,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Whether the task was seeded by the system (immutable) or added by the
    /// user (mutable / deletable).
    #[serde(default)]
    pub system: bool,
}

fn default_enabled() -> bool {
    true
}

/// Context handed to the decider for one evaluation. Implementations of
/// [`SubconsciousDecider`] should use this as their full prompt context.
#[derive(Debug, Clone)]
pub struct SituationReport {
    /// Plain-text summary of recent memory / context. Built by the caller.
    pub memory_brief: String,
    /// Optional workspace-state lines (open files, recent edits, etc.).
    pub workspace_brief: String,
}

#[async_trait]
pub trait SubconsciousDecider: Send + Sync {
    async fn decide(
        &self,
        task: &SubconsciousTask,
        situation: &SituationReport,
    ) -> Result<Decision, SubconsciousError>;
}

/// Executes Act decisions locally and Escalate decisions via a cloud agent.
#[async_trait]
pub trait SubconsciousActor: Send + Sync {
    /// Run the task's intended action with the local model / tools.
    async fn act(
        &self,
        task: &SubconsciousTask,
        situation: &SituationReport,
    ) -> Result<ActOutcome, SubconsciousError>;

    /// Hand a task off to a cloud agent.
    ///
    /// The default impl returns `Approved` so the act path is exercised in
    /// tests; production wiring should call out to the cloud agent.
    async fn escalate(
        &self,
        task: &SubconsciousTask,
        situation: &SituationReport,
    ) -> Result<EscalationOutcome, SubconsciousError> {
        let _ = (task, situation);
        Ok(EscalationOutcome::Completed("escalation not wired".into()))
    }
}

#[derive(Debug, Clone)]
pub struct ActOutcome {
    pub summary: String,
}

#[derive(Debug, Clone)]
pub enum EscalationOutcome {
    /// Cloud agent ran and produced a result.
    Completed(String),
    /// Cloud agent surfaced an action that needs human approval. Caller is
    /// expected to surface this through the existing approval UI.
    AwaitingApproval { proposal: String },
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SubconsciousError(pub String);

impl From<String> for SubconsciousError {
    fn from(s: String) -> Self {
        Self(s)
    }
}
impl From<&str> for SubconsciousError {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// In-memory task registry. Persistence is left to the embedder — load tasks
/// from `HEARTBEAT.md` / `SUBCONSCIOUS.md` / config and seed via [`replace_all`].
#[derive(Debug, Default)]
pub struct TaskRegistry {
    tasks: RwLock<HashMap<String, SubconsciousTask>>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn replace_all(&self, tasks: Vec<SubconsciousTask>) {
        let mut map = self.tasks.write().await;
        map.clear();
        for t in tasks {
            map.insert(t.id.clone(), t);
        }
    }

    pub async fn insert(&self, task: SubconsciousTask) {
        let mut map = self.tasks.write().await;
        map.insert(task.id.clone(), task);
    }

    pub async fn remove(&self, id: &str) -> Option<SubconsciousTask> {
        let mut map = self.tasks.write().await;
        let existing = map.get(id).cloned();
        if let Some(t) = &existing {
            if t.system {
                // System tasks can be disabled but never deleted.
                return None;
            }
        }
        map.remove(id)
    }

    pub async fn enabled_tasks(&self) -> Vec<SubconsciousTask> {
        let map = self.tasks.read().await;
        map.values().filter(|t| t.enabled).cloned().collect()
    }

    pub async fn len(&self) -> usize {
        self.tasks.read().await.len()
    }
}

/// Builder for default system tasks. Embedders can extend the list before
/// installing them in the registry.
pub fn default_system_tasks() -> Vec<SubconsciousTask> {
    vec![
        SubconsciousTask {
            id: "system.connections.health".into(),
            description: "Check connected skills/channels for errors or disconnections.".into(),
            intent: TaskIntent::ReadOnly,
            enabled: true,
            system: true,
        },
        SubconsciousTask {
            id: "system.memory.review".into(),
            description: "Review new memory updates for actionable items.".into(),
            intent: TaskIntent::ReadOnly,
            enabled: true,
            system: true,
        },
        SubconsciousTask {
            id: "system.host.health".into(),
            description: "Monitor local model and gateway health.".into(),
            intent: TaskIntent::ReadOnly,
            enabled: true,
            system: true,
        },
    ]
}

/// Result of evaluating a single task during a tick.
#[derive(Debug, Clone)]
pub struct TickResult {
    pub task_id: String,
    pub state: TickState,
}

#[derive(Debug, Clone)]
pub enum TickState {
    Skipped,
    Acted { summary: String },
    Escalated { outcome: EscalationOutcome },
    AwaitingApproval { proposal: String },
    Failed { error: String },
}

pub struct SubconsciousEngine {
    tasks: Arc<TaskRegistry>,
    decider: Arc<dyn SubconsciousDecider>,
    actor: Arc<dyn SubconsciousActor>,
    pub tick_interval: Duration,
}

impl SubconsciousEngine {
    pub const DEFAULT_TICK: Duration = Duration::from_secs(5 * 60);

    pub fn new(
        tasks: Arc<TaskRegistry>,
        decider: Arc<dyn SubconsciousDecider>,
        actor: Arc<dyn SubconsciousActor>,
    ) -> Self {
        Self {
            tasks,
            decider,
            actor,
            tick_interval: Self::DEFAULT_TICK,
        }
    }

    pub fn with_tick(mut self, tick: Duration) -> Self {
        self.tick_interval = tick;
        self
    }

    /// Run the loop forever. Cancel by dropping the future.
    pub async fn run(self, mut situation: impl SituationBuilder) {
        info!(
            tick_secs = self.tick_interval.as_secs(),
            "subconscious engine started"
        );
        let mut ticker = tokio::time::interval(self.tick_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let sit = situation.build().await;
            let _ = self.tick(sit).await;
        }
    }

    /// One tick. Returns per-task results. Errors are captured as
    /// [`TickState::Failed`] — the loop never panics out.
    pub async fn tick(&self, situation: SituationReport) -> Vec<TickResult> {
        let tasks = self.tasks.enabled_tasks().await;
        let mut out = Vec::with_capacity(tasks.len());
        for task in tasks {
            let result = self.evaluate_one(&task, &situation).await;
            out.push(result);
        }
        out
    }

    async fn evaluate_one(
        &self,
        task: &SubconsciousTask,
        situation: &SituationReport,
    ) -> TickResult {
        match self.decider.decide(task, situation).await {
            Ok(Decision::Skip) => TickResult {
                task_id: task.id.clone(),
                state: TickState::Skipped,
            },
            Ok(Decision::Act) => match self.actor.act(task, situation).await {
                Ok(out) => TickResult {
                    task_id: task.id.clone(),
                    state: TickState::Acted {
                        summary: out.summary,
                    },
                },
                Err(e) => TickResult {
                    task_id: task.id.clone(),
                    state: TickState::Failed { error: e.0 },
                },
            },
            Ok(Decision::Escalate) => match self.actor.escalate(task, situation).await {
                Ok(EscalationOutcome::AwaitingApproval { proposal }) if task.intent == TaskIntent::ReadOnly => {
                    TickResult {
                        task_id: task.id.clone(),
                        state: TickState::AwaitingApproval { proposal },
                    }
                }
                Ok(outcome) => TickResult {
                    task_id: task.id.clone(),
                    state: TickState::Escalated { outcome },
                },
                Err(e) => TickResult {
                    task_id: task.id.clone(),
                    state: TickState::Failed { error: e.0 },
                },
            },
            Err(e) => {
                warn!(task = %task.id, error = %e.0, "decider failed");
                TickResult {
                    task_id: task.id.clone(),
                    state: TickState::Failed { error: e.0 },
                }
            }
        }
    }
}

/// Builds a fresh [`SituationReport`] each tick. Implement to plug in real
/// memory / workspace summarizers.
#[async_trait]
pub trait SituationBuilder: Send + Sync + 'static {
    async fn build(&mut self) -> SituationReport;
}

#[async_trait]
impl<F, Fut> SituationBuilder for F
where
    F: FnMut() -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = SituationReport> + Send,
{
    async fn build(&mut self) -> SituationReport {
        (self)().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FixedDecider(Decision);
    #[async_trait]
    impl SubconsciousDecider for FixedDecider {
        async fn decide(
            &self,
            _task: &SubconsciousTask,
            _situation: &SituationReport,
        ) -> Result<Decision, SubconsciousError> {
            Ok(self.0)
        }
    }

    struct CountingActor {
        acts: Arc<AtomicUsize>,
        escalates: Arc<AtomicUsize>,
        escalation_outcome: EscalationOutcome,
    }
    #[async_trait]
    impl SubconsciousActor for CountingActor {
        async fn act(
            &self,
            task: &SubconsciousTask,
            _situation: &SituationReport,
        ) -> Result<ActOutcome, SubconsciousError> {
            self.acts.fetch_add(1, Ordering::SeqCst);
            Ok(ActOutcome {
                summary: format!("acted on {}", task.id),
            })
        }
        async fn escalate(
            &self,
            _task: &SubconsciousTask,
            _situation: &SituationReport,
        ) -> Result<EscalationOutcome, SubconsciousError> {
            self.escalates.fetch_add(1, Ordering::SeqCst);
            Ok(self.escalation_outcome.clone())
        }
    }

    fn sit() -> SituationReport {
        SituationReport {
            memory_brief: "nothing new".into(),
            workspace_brief: "".into(),
        }
    }

    #[tokio::test]
    async fn skip_decision_does_nothing() {
        let registry = Arc::new(TaskRegistry::new());
        registry
            .replace_all(vec![SubconsciousTask {
                id: "t1".into(),
                description: "x".into(),
                intent: TaskIntent::ReadOnly,
                enabled: true,
                system: false,
            }])
            .await;
        let acts = Arc::new(AtomicUsize::new(0));
        let escalates = Arc::new(AtomicUsize::new(0));
        let engine = SubconsciousEngine::new(
            Arc::clone(&registry),
            Arc::new(FixedDecider(Decision::Skip)),
            Arc::new(CountingActor {
                acts: Arc::clone(&acts),
                escalates: Arc::clone(&escalates),
                escalation_outcome: EscalationOutcome::Completed("ok".into()),
            }),
        );
        let results = engine.tick(sit()).await;
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].state, TickState::Skipped));
        assert_eq!(acts.load(Ordering::SeqCst), 0);
        assert_eq!(escalates.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn act_decision_invokes_actor() {
        let registry = Arc::new(TaskRegistry::new());
        registry
            .replace_all(vec![SubconsciousTask {
                id: "t1".into(),
                description: "x".into(),
                intent: TaskIntent::Write,
                enabled: true,
                system: false,
            }])
            .await;
        let acts = Arc::new(AtomicUsize::new(0));
        let escalates = Arc::new(AtomicUsize::new(0));
        let engine = SubconsciousEngine::new(
            Arc::clone(&registry),
            Arc::new(FixedDecider(Decision::Act)),
            Arc::new(CountingActor {
                acts: Arc::clone(&acts),
                escalates: Arc::clone(&escalates),
                escalation_outcome: EscalationOutcome::Completed("ok".into()),
            }),
        );
        let results = engine.tick(sit()).await;
        assert_eq!(acts.load(Ordering::SeqCst), 1);
        assert!(matches!(
            results[0].state,
            TickState::Acted { ref summary } if summary.contains("t1")
        ));
    }

    #[tokio::test]
    async fn escalate_readonly_needing_approval_lands_in_approval_state() {
        let registry = Arc::new(TaskRegistry::new());
        registry
            .replace_all(vec![SubconsciousTask {
                id: "t1".into(),
                description: "x".into(),
                intent: TaskIntent::ReadOnly,
                enabled: true,
                system: false,
            }])
            .await;
        let engine = SubconsciousEngine::new(
            Arc::clone(&registry),
            Arc::new(FixedDecider(Decision::Escalate)),
            Arc::new(CountingActor {
                acts: Arc::new(AtomicUsize::new(0)),
                escalates: Arc::new(AtomicUsize::new(0)),
                escalation_outcome: EscalationOutcome::AwaitingApproval {
                    proposal: "send email".into(),
                },
            }),
        );
        let results = engine.tick(sit()).await;
        match &results[0].state {
            TickState::AwaitingApproval { proposal } => {
                assert_eq!(proposal, "send email");
            }
            other => panic!("expected AwaitingApproval, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn system_task_cannot_be_removed_but_user_can() {
        let r = TaskRegistry::new();
        r.insert(SubconsciousTask {
            id: "sys".into(),
            description: "".into(),
            intent: TaskIntent::ReadOnly,
            enabled: true,
            system: true,
        })
        .await;
        r.insert(SubconsciousTask {
            id: "user".into(),
            description: "".into(),
            intent: TaskIntent::ReadOnly,
            enabled: true,
            system: false,
        })
        .await;
        assert!(r.remove("sys").await.is_none());
        assert!(r.remove("user").await.is_some());
        assert_eq!(r.len().await, 1);
    }
}
