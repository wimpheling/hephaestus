//! `PostgreSQL` persistence adapter for durable runs.

mod cleanup;
mod cleanup_wake;
#[path = "run_postgres/create.rs"]
mod create;
#[path = "run_postgres/errors.rs"]
mod errors;
mod execution_profile;
mod instance_drain;
mod invocation_admission;
mod legacy_placement;
#[path = "run_postgres/model.rs"]
mod model;
#[path = "run_postgres/operations.rs"]
mod operations;
mod ordinary_queued_cancel;
mod parent_transition;
mod planned_create;
mod profiled_lifecycle;
#[path = "run_postgres/provenance.rs"]
mod provenance;
mod qualified_invocation_routing;
mod queued_cancel_delivery;
mod queued_cleanup;
#[path = "run_postgres/repository.rs"]
mod repository;
mod runtime_catalog;
mod vm_owner_history;
pub use vm_owner_history::PgVmOwnerHistoryReader;

pub use cleanup::PgRunCleanupRepository;
pub use instance_drain::{
    InstanceDrainOriginal, InstanceDrainRunObservation, PgInstanceDrainReader,
};
pub use profiled_lifecycle::{ProfiledRunRecoveryHold, ProfiledRunRecoveryHoldReason};
pub use qualified_invocation_routing::{
    PgQualifiedInvocationRoutingReader, QualifiedInvocationEvidence, QualifiedInvocationRoute,
    QualifiedInvocationRoutingDecision, QualifiedInvocationRoutingHoldReason,
    QualifiedOrdinaryEvidence, QualifiedOrdinaryGitSource, QualifiedOrdinaryRoute,
    QualifiedOrdinaryRoutingDecision, QualifiedOrdinaryRoutingHoldReason, QualifiedOrdinaryUnborn,
};
pub use repository::PgRunRepository;
