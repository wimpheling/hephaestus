//! Real `PostgreSQL` coverage for bounded service-invocation recovery.

#[path = "service_recovery/scenarios.rs"]
mod scenarios;
#[path = "service_recovery/support_database.rs"]
mod support_database;
#[path = "service_recovery/support_invocation.rs"]
mod support_invocation;
#[path = "service_recovery/support_lease.rs"]
mod support_lease;
#[path = "service_recovery/support_seed.rs"]
mod support_seed;
#[path = "service_recovery/support_types.rs"]
mod support_types;
