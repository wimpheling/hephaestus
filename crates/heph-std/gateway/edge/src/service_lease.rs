//! Caller-owned durable lease renewal for one persistent gateway service.

mod monitor;
mod types;
mod validation;

#[cfg(test)]
#[path = "service_lease/tests.rs"]
mod tests;

pub use monitor::GatewayServiceLeaseMonitor;
pub use types::{
    GatewayServiceLeaseControl, GatewayServiceLeaseError, GatewayServiceLeaseLossReason,
    GatewayServiceLeasePolicy, GatewayServiceLeaseRetryReason, GatewayServiceLeaseRunResult,
    GatewayServiceLeaseStatus,
};
