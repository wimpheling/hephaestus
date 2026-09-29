//! Typed authorization values and transaction-aware provider contract.

mod decision_error;
mod model;
mod permission;
mod traits;

pub use decision_error::{AuthorizationDecision, AuthzError};
pub use model::{ObjectRef, ObjectType, Subject};
pub use permission::Permission;
pub use traits::{GitRepositoryAuthorizer, GitRepositoryOperation};

#[cfg(test)]
#[path = "tests/authz.rs"]
mod tests;
