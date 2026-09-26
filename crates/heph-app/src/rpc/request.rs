//! Shared authenticated RPC request conversion.

#[path = "request/budget.rs"]
mod budget;
#[path = "request/identity.rs"]
mod identity;

pub use budget::{RequestBudget, run_with_budget};
pub use identity::{mutation_identity, query_identity, required_id, verified_mediator_session};

#[cfg(test)]
use super::{MediatorAuthenticator, RpcError};
#[cfg(test)]
use budget::test_budget;
#[cfg(test)]
use identity::{MAX_IDEMPOTENCY_KEY_BYTES, derive_idempotency_id, mutation_request_id};

#[cfg(test)]
#[path = "request/tests.rs"]
mod tests;
