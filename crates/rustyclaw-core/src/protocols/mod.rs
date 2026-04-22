//! Shared protocols for agent coordination and reliability.
//!
//! Inspired by production-grade agent systems, these protocols provide:
//! - **Receipt Protocol**: Verifiable task completion with artifacts and metrics
//! - **Freshness Protocol**: Data volatility awareness to prevent stale information

pub mod freshness;
pub mod receipt;

pub use freshness::{FreshnessProtocol, VolatilityTier};
pub use receipt::{ReceiptStore, TaskReceipt};
