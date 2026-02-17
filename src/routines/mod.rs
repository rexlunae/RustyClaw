//! Routines engine for scheduled and event-driven automation.

pub mod cron_scheduler;
pub mod engine;
pub mod event_matcher;
pub mod store;

// Re-export main types
pub use cron_scheduler::CronScheduler;
pub use engine::{RoutineChecker, RoutineEngine, RoutineExecutor};
pub use event_matcher::{Event, EventDispatcher, EventMatcher};
pub use store::{
    ExecutionStatus, Routine, RoutineExecution, RoutinesConfig, RoutinesStore, TriggerConfig,
    TriggerType,
};
