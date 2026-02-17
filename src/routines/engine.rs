//! Routine execution engine with guardrails.
//!
//! Orchestrates routine execution by checking schedules, matching events,
//! and enforcing safety guardrails.

use super::{
    CronScheduler, Event, EventDispatcher, Routine, RoutineExecution, RoutinesStore,
    TriggerConfig, TriggerType,
};
use anyhow::{Context as AnyhowContext, Result};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Routine execution engine.
pub struct RoutineEngine {
    store: Arc<RoutinesStore>,
    event_dispatcher: Arc<RwLock<EventDispatcher>>,
    /// Callback for executing routine prompts
    executor: Option<Arc<dyn RoutineExecutor>>,
}

/// Trait for executing routine prompts.
///
/// Implementations should integrate with the AI agent to execute the prompt
/// and return the result.
#[async_trait::async_trait]
pub trait RoutineExecutor: Send + Sync {
    /// Execute a routine prompt and return the result.
    async fn execute_prompt(&self, routine: &Routine) -> Result<String>;
}

impl RoutineEngine {
    /// Create a new routine engine.
    pub fn new(store: Arc<RoutinesStore>) -> Self {
        Self {
            store,
            event_dispatcher: Arc::new(RwLock::new(EventDispatcher::new())),
            executor: None,
        }
    }

    /// Set the routine executor.
    pub fn with_executor(mut self, executor: Arc<dyn RoutineExecutor>) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Load all enabled routines and initialize event matchers.
    pub async fn initialize(&self) -> Result<()> {
        let routines = self.store.list_routines(true).await?;
        let mut dispatcher = self.event_dispatcher.write().await;

        dispatcher.clear();

        for routine in routines {
            if routine.trigger_type == TriggerType::Event {
                if let TriggerConfig::Event { pattern } = &routine.trigger_config {
                    let matcher = super::EventMatcher::new(pattern)
                        .with_context(|| {
                            format!("Failed to create matcher for routine: {}", routine.name)
                        })?;

                    let routine_id = routine
                        .id
                        .ok_or_else(|| anyhow::anyhow!("Routine has no ID"))?;
                    dispatcher.register(routine_id.to_string(), matcher);
                }
            }
        }

        Ok(())
    }

    /// Check for routines that should run now based on cron schedules.
    ///
    /// Returns a list of routine IDs that should be executed.
    pub async fn check_cron_schedules(&self) -> Result<Vec<i64>> {
        let routines = self.store.list_routines(true).await?;
        let mut ready_routines = Vec::new();

        for routine in routines {
            if routine.trigger_type != TriggerType::Cron {
                continue;
            }

            // Check if routine is in cooldown
            if routine.is_in_cooldown() {
                continue;
            }

            // Check if routine should be disabled
            if routine.should_disable() {
                continue;
            }

            // Parse cron schedule
            if let TriggerConfig::Cron { expression } = &routine.trigger_config {
                let scheduler = CronScheduler::new(expression)
                    .with_context(|| {
                        format!("Failed to parse cron expression for routine: {}", routine.name)
                    })?;

                if scheduler.should_run_now() {
                    if let Some(id) = routine.id {
                        ready_routines.push(id);
                    }
                }
            }
        }

        Ok(ready_routines)
    }

    /// Dispatch an event and return routine IDs that should be triggered.
    pub async fn dispatch_event(&self, event: &Event) -> Result<Vec<i64>> {
        let dispatcher = self.event_dispatcher.read().await;
        let routine_ids: Vec<i64> = dispatcher
            .dispatch(event)
            .iter()
            .filter_map(|id_str| id_str.parse::<i64>().ok())
            .collect();

        // Filter out routines that are in cooldown or should be disabled
        let mut filtered_ids = Vec::new();
        for id in routine_ids {
            if let Some(routine) = self.store.get_routine(id).await? {
                if !routine.is_in_cooldown() && !routine.should_disable() {
                    filtered_ids.push(id);
                }
            }
        }

        Ok(filtered_ids)
    }

    /// Execute a routine by ID.
    ///
    /// This method:
    /// 1. Loads the routine from storage
    /// 2. Creates an execution record
    /// 3. Executes the prompt via the executor
    /// 4. Updates execution status and routine metadata
    /// 5. Enforces guardrails (max failures)
    pub async fn execute_routine(
        &self,
        routine_id: i64,
        trigger_source: Option<String>,
    ) -> Result<()> {
        // Load routine
        let mut routine = self
            .store
            .get_routine(routine_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Routine not found: {}", routine_id))?;

        // Check if disabled
        if !routine.enabled {
            anyhow::bail!("Routine is disabled: {}", routine.name);
        }

        // Create execution record
        let execution = RoutineExecution::new(routine_id, trigger_source.clone());
        let execution_id = self.store.create_execution(execution).await?;

        // Execute the prompt
        let result = if let Some(executor) = &self.executor {
            executor.execute_prompt(&routine).await
        } else {
            Err(anyhow::anyhow!("No executor configured"))
        };

        // Update execution record
        let mut execution = RoutineExecution {
            id: Some(execution_id),
            routine_id,
            started_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            status: super::ExecutionStatus::Running,
            trigger_source,
            error_message: None,
            output: None,
        };

        match result {
            Ok(output) => {
                execution = execution.mark_success(Some(output));
                routine.last_success = Some(chrono::Utc::now().timestamp());
                routine.failure_count = 0; // Reset failure count on success
            }
            Err(e) => {
                execution = execution.mark_failed(e.to_string());
                routine.failure_count += 1;
            }
        }

        // Update last run timestamp
        routine.last_run = Some(chrono::Utc::now().timestamp());

        // Disable routine if too many failures
        if routine.should_disable() {
            routine.enabled = false;
        }

        // Save updates
        self.store.update_execution(&execution).await?;
        self.store.update_routine(&routine).await?;

        Ok(())
    }

    /// Execute a routine by name.
    pub async fn execute_routine_by_name(
        &self,
        name: &str,
        trigger_source: Option<String>,
    ) -> Result<()> {
        let routine = self
            .store
            .get_routine_by_name(name)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Routine not found: {}", name))?;

        let routine_id = routine
            .id
            .ok_or_else(|| anyhow::anyhow!("Routine has no ID"))?;

        self.execute_routine(routine_id, trigger_source).await
    }

    /// Get the routine store.
    pub fn store(&self) -> &Arc<RoutinesStore> {
        &self.store
    }

    /// Get a read-only reference to the event dispatcher.
    pub async fn event_dispatcher(&self) -> tokio::sync::RwLockReadGuard<'_, EventDispatcher> {
        self.event_dispatcher.read().await
    }
}

/// Background routine checker that periodically checks for pending routines.
pub struct RoutineChecker {
    engine: Arc<RoutineEngine>,
    check_interval_secs: u64,
}

impl RoutineChecker {
    /// Create a new routine checker.
    pub fn new(engine: Arc<RoutineEngine>, check_interval_secs: u64) -> Self {
        Self {
            engine,
            check_interval_secs,
        }
    }

    /// Start the background checker loop.
    ///
    /// This runs indefinitely and should be spawned as a tokio task.
    pub async fn run(&self) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(
            self.check_interval_secs,
        ));

        loop {
            interval.tick().await;

            if let Err(e) = self.check_and_execute().await {
                eprintln!("[RoutineChecker] Error checking routines: {}", e);
            }
        }
    }

    /// Check for pending routines and execute them.
    async fn check_and_execute(&self) -> Result<()> {
        // Check cron schedules
        let ready_routines = self.engine.check_cron_schedules().await?;

        for routine_id in ready_routines {
            // Execute sequentially (routines should be relatively infrequent)
            if let Err(e) = self
                .engine
                .execute_routine(routine_id, Some("cron".to_string()))
                .await
            {
                eprintln!(
                    "[RoutineChecker] Failed to execute routine {}: {}",
                    routine_id, e
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routines::{RoutinesConfig, TriggerConfig};
    use tempfile::TempDir;

    struct MockExecutor;

    #[async_trait::async_trait]
    impl RoutineExecutor for MockExecutor {
        async fn execute_prompt(&self, routine: &Routine) -> Result<String> {
            // Simulate execution
            Ok(format!("Executed: {}", routine.prompt))
        }
    }

    #[tokio::test]
    async fn test_engine_initialization() {
        let temp = TempDir::new().unwrap();
        let config = RoutinesConfig::default();
        let store = Arc::new(RoutinesStore::open(temp.path(), config).unwrap());

        // Create a routine with event trigger
        let routine = Routine::new(
            "test-routine".to_string(),
            "Test prompt".to_string(),
            TriggerConfig::Event {
                pattern: r"(?i)error".to_string(),
            },
        );
        store.create_routine(routine).await.unwrap();

        let engine = RoutineEngine::new(store);
        engine.initialize().await.unwrap();

        let dispatcher = engine.event_dispatcher().await;
        assert_eq!(dispatcher.matcher_count(), 1);
    }

    #[tokio::test]
    async fn test_event_dispatch() {
        let temp = TempDir::new().unwrap();
        let config = RoutinesConfig::default();
        let store = Arc::new(RoutinesStore::open(temp.path(), config).unwrap());

        // Create a routine
        let routine = Routine::new(
            "error-routine".to_string(),
            "Handle error".to_string(),
            TriggerConfig::Event {
                pattern: r"(?i)error".to_string(),
            },
        );
        let routine_id = store.create_routine(routine).await.unwrap();

        let engine = RoutineEngine::new(store);
        engine.initialize().await.unwrap();

        // Dispatch an error event
        let event = Event::agent_response("An error occurred");
        let triggered = engine.dispatch_event(&event).await.unwrap();

        assert_eq!(triggered.len(), 1);
        assert_eq!(triggered[0], routine_id);
    }

    #[tokio::test]
    async fn test_routine_execution() {
        let temp = TempDir::new().unwrap();
        let config = RoutinesConfig::default();
        let store = Arc::new(RoutinesStore::open(temp.path(), config).unwrap());

        // Create a manual routine
        let routine = Routine::new(
            "manual-routine".to_string(),
            "Test prompt".to_string(),
            TriggerConfig::Manual,
        );
        let routine_id = store.create_routine(routine).await.unwrap();

        let engine = Arc::new(
            RoutineEngine::new(store.clone()).with_executor(Arc::new(MockExecutor)),
        );

        // Execute the routine
        engine
            .execute_routine(routine_id, Some("manual".to_string()))
            .await
            .unwrap();

        // Check execution history
        let executions = store.get_executions(routine_id, 10).await.unwrap();
        assert_eq!(executions.len(), 1);
        assert_eq!(executions[0].status, super::super::ExecutionStatus::Success);

        // Check routine was updated
        let routine = store.get_routine(routine_id).await.unwrap().unwrap();
        assert!(routine.last_run.is_some());
        assert!(routine.last_success.is_some());
        assert_eq!(routine.failure_count, 0);
    }

    #[tokio::test]
    async fn test_failure_count_increment() {
        let temp = TempDir::new().unwrap();
        let config = RoutinesConfig::default();
        let store = Arc::new(RoutinesStore::open(temp.path(), config).unwrap());

        struct FailingExecutor;

        #[async_trait::async_trait]
        impl RoutineExecutor for FailingExecutor {
            async fn execute_prompt(&self, _routine: &Routine) -> Result<String> {
                anyhow::bail!("Simulated failure")
            }
        }

        let routine = Routine::new(
            "failing-routine".to_string(),
            "Test prompt".to_string(),
            TriggerConfig::Manual,
        )
        .with_max_failures(3);
        let routine_id = store.create_routine(routine).await.unwrap();

        let engine = Arc::new(
            RoutineEngine::new(store.clone()).with_executor(Arc::new(FailingExecutor)),
        );

        // Execute multiple times to trigger failures
        for _ in 0..3 {
            let _ = engine
                .execute_routine(routine_id, Some("manual".to_string()))
                .await;
        }

        // Check routine was disabled after max failures
        let routine = store.get_routine(routine_id).await.unwrap().unwrap();
        assert_eq!(routine.failure_count, 3);
        assert!(!routine.enabled); // Should be disabled
    }

    #[tokio::test]
    async fn test_cooldown_enforcement() {
        let temp = TempDir::new().unwrap();
        let config = RoutinesConfig::default();
        let store = Arc::new(RoutinesStore::open(temp.path(), config).unwrap());

        let mut routine = Routine::new(
            "cooldown-routine".to_string(),
            "Test prompt".to_string(),
            TriggerConfig::Event {
                pattern: r"test".to_string(),
            },
        )
        .with_cooldown(3600); // 1 hour cooldown

        // Set last run to now
        routine.last_run = Some(chrono::Utc::now().timestamp());
        let routine_id = store.create_routine(routine).await.unwrap();

        let engine = RoutineEngine::new(store);
        engine.initialize().await.unwrap();

        // Try to trigger via event
        let event = Event::agent_response("test message");
        let triggered = engine.dispatch_event(&event).await.unwrap();

        // Should be empty due to cooldown
        assert_eq!(triggered.len(), 0);
    }
}
