//! Cron expression parsing and scheduling for routines.
//!
//! Provides cron-based triggering with next execution time calculation.

use anyhow::{Context as AnyhowContext, Result};
use chrono::{DateTime, Utc};
use cron::Schedule;
use std::str::FromStr;

/// Cron scheduler for time-based routine triggers.
pub struct CronScheduler {
    schedule: Schedule,
}

impl CronScheduler {
    /// Create a new cron scheduler from an expression.
    ///
    /// Accepts standard cron syntax:
    /// ```text
    /// ┌───────────── minute (0 - 59)
    /// │ ┌───────────── hour (0 - 23)
    /// │ │ ┌───────────── day of month (1 - 31)
    /// │ │ │ ┌───────────── month (1 - 12)
    /// │ │ │ │ ┌───────────── day of week (0 - 6, Sunday = 0)
    /// │ │ │ │ │
    /// * * * * *
    /// ```
    ///
    /// Examples:
    /// - `"0 9 * * *"` - Every day at 9:00 AM
    /// - `"0 9 * * MON-FRI"` - Weekdays at 9:00 AM
    /// - `"*/15 * * * *"` - Every 15 minutes
    /// - `"0 0 1 * *"` - First day of every month at midnight
    pub fn new(expression: &str) -> Result<Self> {
        let schedule = Schedule::from_str(expression)
            .with_context(|| format!("Invalid cron expression: {}", expression))?;

        Ok(Self { schedule })
    }

    /// Get the next execution time after the given datetime.
    pub fn next_execution(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.schedule.after(&after).next()
    }

    /// Get the next execution time from now.
    pub fn next_execution_from_now(&self) -> Option<DateTime<Utc>> {
        self.next_execution(Utc::now())
    }

    /// Check if the routine should run now (within the last minute).
    ///
    /// Returns true if the next scheduled time is in the past or within
    /// the next 60 seconds (to handle check interval jitter).
    pub fn should_run_now(&self) -> bool {
        if let Some(next) = self.next_execution_from_now() {
            let now = Utc::now();
            let seconds_until = (next - now).num_seconds();
            // Run if scheduled time has passed or is within next 60 seconds
            seconds_until <= 60
        } else {
            false
        }
    }

    /// Get the cron expression string.
    pub fn expression(&self) -> String {
        format!("{}", self.schedule)
    }

    /// Validate a cron expression without creating a scheduler.
    pub fn validate(expression: &str) -> Result<()> {
        Schedule::from_str(expression)
            .with_context(|| format!("Invalid cron expression: {}", expression))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_daily_schedule() {
        let scheduler = CronScheduler::new("0 9 * * *").unwrap();

        // Get next execution from a specific time
        let now = Utc.with_ymd_and_hms(2026, 2, 17, 8, 0, 0).unwrap();
        let next = scheduler.next_execution(now).unwrap();

        assert_eq!(next.hour(), 9);
        assert_eq!(next.minute(), 0);
    }

    #[test]
    fn test_weekday_schedule() {
        let scheduler = CronScheduler::new("0 9 * * MON-FRI").unwrap();

        // Monday 2026-02-17
        let monday = Utc.with_ymd_and_hms(2026, 2, 17, 8, 0, 0).unwrap();
        let next = scheduler.next_execution(monday).unwrap();
        assert_eq!(next.hour(), 9);

        // Should skip Saturday and run on Monday
        let saturday = Utc.with_ymd_and_hms(2026, 2, 21, 10, 0, 0).unwrap();
        let next_after_weekend = scheduler.next_execution(saturday).unwrap();
        assert_eq!(next_after_weekend.weekday(), chrono::Weekday::Mon);
    }

    #[test]
    fn test_every_15_minutes() {
        let scheduler = CronScheduler::new("*/15 * * * *").unwrap();

        let now = Utc.with_ymd_and_hms(2026, 2, 17, 10, 7, 0).unwrap();
        let next = scheduler.next_execution(now).unwrap();

        // Should be 10:15
        assert_eq!(next.hour(), 10);
        assert_eq!(next.minute(), 15);
    }

    #[test]
    fn test_invalid_expression() {
        let result = CronScheduler::new("invalid cron");
        assert!(result.is_err());
    }

    #[test]
    fn test_validation() {
        assert!(CronScheduler::validate("0 9 * * *").is_ok());
        assert!(CronScheduler::validate("*/5 * * * *").is_ok());
        assert!(CronScheduler::validate("invalid").is_err());
    }

    #[test]
    fn test_should_run_now() {
        // Create a schedule that runs every minute
        let scheduler = CronScheduler::new("* * * * *").unwrap();

        // Should always be ready to run (within next 60 seconds)
        assert!(scheduler.should_run_now());
    }

    #[test]
    fn test_expression_display() {
        let scheduler = CronScheduler::new("0 9 * * MON-FRI").unwrap();
        let expr = scheduler.expression();
        assert!(expr.contains("9") || expr.contains("MON") || expr.contains("0 9"));
    }
}
