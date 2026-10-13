//! Owning strict Legacy composition over the existing managed backend.

mod assemble;
mod inputs;
mod paths;
mod producers;
mod roots;
mod secrets;
mod validation;
mod workspace_validation;

use crate::{AppError, component};
use heph_run::{CancelRun, RepositoryError, Run, RunRepository, StartRun};
use heph_runtime::RootFilesystem;
pub use inputs::{ManagedLegacyAuthority, ManagedLegacyInputs, ManagedLegacyPools};
use run_domain::{LegacyVmPlacementInventory, LegacyVmPlacementScope};
use run_orchestrator::{OrchestratorError, RunOrchestrator};
use run_postgres::PgRunRepository;
use std::sync::Arc;

/// Owns one strict Legacy controller and its concrete producer components.
///
/// All controller families must receive the SAME backend and operation guards.
/// This handle does not route, recover, start services, or reopen stopped gates.
/// `PostgreSQL` Legacy Wake is unsupported and intentionally has no delegate.
/// Drop establishes neither stopped nor cleaned state; backend custody remains
/// the caller's responsibility, including constructor errors and cancellation.
pub struct ManagedLegacyController {
    // Retain the actual typed provider, owners, guards, pools and trusted inputs.
    retained: ManagedLegacyInputs,
    _producers: producers::Producers,
    scope: LegacyVmPlacementScope,
    runs: Arc<PgRunRepository>,
    orchestrator: Arc<RunOrchestrator>,
}

impl ManagedLegacyController {
    /// Validates actual bindings and composes the ordinary production adapters.
    ///
    /// Read-only checks, the actual three-pool startup preflight, and repeated
    /// read-only checks precede the first filesystem initializer. The checked
    /// configuration is comparison data, never an actor grant or readiness proof.
    /// Core and `PostgreSQL` independently requalify each later operation.
    ///
    /// # Errors
    /// Rejects configuration/owner/root/socket/policy mismatches, incompatible
    /// roles/schema/service, or failed initialization. Initializer errors may
    /// leave directories; this constructor claims no automatic cleanup.
    pub async fn new(inputs: ManagedLegacyInputs) -> Result<Self, AppError> {
        validation::validate(&inputs)?;
        control_plane_postgres::verify_qualified_invocation_contract(
            &inputs.pools.admin,
            &inputs.pools.worker,
            &inputs.pools.application,
        )
        .await
        .map_err(component("managed Legacy database startup"))?;
        validation::validate(&inputs)?;
        let composition = assemble::build(&inputs).await?;
        Ok(Self {
            retained: inputs,
            _producers: composition.producers,
            scope: composition.scope,
            runs: composition.runs,
            orchestrator: composition.orchestrator,
        })
    }

    /// Starts the exact command through strict original placement qualification.
    ///
    /// Retains this owning controller through the awaited core operation.
    ///
    /// # Errors
    /// Propagates original-producer, owner, authorization, resource or VM errors.
    pub async fn start_run(self: Arc<Self>, command: &StartRun) -> Result<Run, OrchestratorError> {
        let result = self.orchestrator.start_run(command).await;
        drop(self);
        result
    }

    /// Cancels the exact original Run through the existing strict Legacy path.
    ///
    /// Retains this owning controller through the awaited core operation.
    ///
    /// # Errors
    /// Propagates original-placement, owner, cleanup or storage errors.
    pub async fn cancel_run(
        self: Arc<Self>,
        command: &CancelRun,
    ) -> Result<bool, OrchestratorError> {
        let result = self.orchestrator.cancel_run(command).await;
        drop(self);
        result
    }

    /// Reads scoped placement inventory, including explicit held evidence.
    ///
    /// This projection grants no IO authority and performs no recovery action.
    ///
    /// # Errors
    /// Propagates worker-role, scope, malformed evidence or database failures.
    pub async fn placement_inventory(&self) -> Result<LegacyVmPlacementInventory, RepositoryError> {
        self.runs.legacy_vm_placement_inventory(&self.scope).await
    }
    /// Same private original handle; no replacement composition or owner access.
    pub(super) fn managed_orchestrator(&self) -> &RunOrchestrator {
        &self.orchestrator
    }

    pub(super) fn validate_owners(&self) -> Result<(), AppError> {
        validation::validate(&self.retained)
    }
}
