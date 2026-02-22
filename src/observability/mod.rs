//! Observability subsystem for agent runtime telemetry.
//!
//! This module provides traits and implementations for recording events and
//! metrics from the agent runtime. The modular design supports multiple backends
//! (console logging, Prometheus, OpenTelemetry) via the [`Observer`] trait.
//!
//! Adapted from ZeroClaw (MIT OR Apache-2.0 licensed).

pub mod log;
pub mod traits;

pub use log::LogObserver;
pub use traits::{Observer, ObserverEvent, ObserverMetric};

use std::sync::Arc;

/// Composite observer that dispatches to multiple backends.
///
/// Useful for sending telemetry to both local logs and external systems
/// (e.g., Prometheus + structured logging).
pub struct CompositeObserver {
    observers: Vec<Arc<dyn Observer>>,
}

impl CompositeObserver {
    /// Create a composite observer from a list of observer implementations.
    pub fn new(observers: Vec<Arc<dyn Observer>>) -> Self {
        Self { observers }
    }

    /// Add an observer to the composite.
    pub fn add(&mut self, observer: Arc<dyn Observer>) {
        self.observers.push(observer);
    }
}

impl Observer for CompositeObserver {
    fn record_event(&self, event: &ObserverEvent) {
        for observer in &self.observers {
            observer.record_event(event);
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        for observer in &self.observers {
            observer.record_metric(metric);
        }
    }

    fn flush(&self) {
        for observer in &self.observers {
            observer.flush();
        }
    }

    fn name(&self) -> &str {
        "composite"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Default)]
    struct CountingObserver {
        events: Mutex<u64>,
        metrics: Mutex<u64>,
        flushes: Mutex<u64>,
    }

    impl Observer for CountingObserver {
        fn record_event(&self, _event: &ObserverEvent) {
            *self.events.lock().unwrap() += 1;
        }

        fn record_metric(&self, _metric: &ObserverMetric) {
            *self.metrics.lock().unwrap() += 1;
        }

        fn flush(&self) {
            *self.flushes.lock().unwrap() += 1;
        }

        fn name(&self) -> &str {
            "counting"
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn composite_dispatches_to_all_backends() {
        let obs1 = Arc::new(CountingObserver::default());
        let obs2 = Arc::new(CountingObserver::default());

        let composite = CompositeObserver::new(vec![obs1.clone(), obs2.clone()]);

        composite.record_event(&ObserverEvent::HeartbeatTick);
        composite.record_metric(&ObserverMetric::TokensUsed(100));
        composite.flush();

        assert_eq!(*obs1.events.lock().unwrap(), 1);
        assert_eq!(*obs2.events.lock().unwrap(), 1);
        assert_eq!(*obs1.metrics.lock().unwrap(), 1);
        assert_eq!(*obs2.metrics.lock().unwrap(), 1);
        assert_eq!(*obs1.flushes.lock().unwrap(), 1);
        assert_eq!(*obs2.flushes.lock().unwrap(), 1);
    }

    #[test]
    fn composite_name() {
        let composite = CompositeObserver::new(vec![]);
        assert_eq!(composite.name(), "composite");
    }
}
