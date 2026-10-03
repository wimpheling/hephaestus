//! Checked placement metadata; neither IO permission nor physical cleanup proof.

use runtime_types::RunId;
use uuid::Uuid;
use vm_trait::{VmId, VmProviderOwnerScope};

use crate::{RunKind, StartRun};

/// The configured persistent owner of a managed Legacy VM provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyVmPlacementScope(VmProviderOwnerScope);

impl LegacyVmPlacementScope {
    /// Checks the actual provider scope, requiring a canonical non-nil UUID owner.
    ///
    /// Shape checks do not discover or establish provider ownership.
    ///
    /// # Errors
    ///
    /// Rejects noncanonical or nil provider namespaces.
    pub fn new(scope: VmProviderOwnerScope) -> Result<Self, LegacyVmPlacementError> {
        let namespace = scope
            .namespace()
            .parse::<RunId>()
            .map_err(|_| LegacyVmPlacementError::Namespace)?;
        if namespace.as_uuid().is_nil() || namespace.to_string() != scope.namespace() {
            return Err(LegacyVmPlacementError::Namespace);
        }
        Ok(Self(scope))
    }

    /// Returns the checked provider scope; callers must still verify ownership.
    #[must_use]
    pub const fn provider_scope(&self) -> &VmProviderOwnerScope {
        &self.0
    }
}

/// Frozen producer relation recorded at the Run's birth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVmPlacementProducer {
    /// Exact normal Run request and start inbox.
    NormalRequest,
    /// Exact accepted mailbox event and delivery attempt.
    Mailbox,
    /// Exact update hook and immutable producer command.
    UpdateHook,
}

/// Permanent worker consumption of positive birth eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVmPlacementConsumption {
    /// May pass separate open-run authorization before resource IO.
    Provision,
    /// Cleanup only; cannot authorize Start, acquire or backing initialization.
    CleanupOnly,
}

/// Exact durable Legacy placement, separate from canonical cleanup receipts.
///
/// This value is a projection. Database provenance and live permissions must be
/// checked by the repository; a constructed DTO is not a physical observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyVmPlacement {
    command: StartRun,
    project: Uuid,
    contract_hash: [u8; 32],
    scope: LegacyVmPlacementScope,
    vm_id: VmId,
    producer: LegacyVmPlacementProducer,
    consumption: Option<LegacyVmPlacementConsumption>,
}

impl LegacyVmPlacement {
    /// Checks exact fresh Run identity and the recorded producer shape.
    ///
    /// # Errors
    ///
    /// Rejects nil pins, a different VM ID, or an incompatible producer kind.
    pub fn new(
        command: StartRun,
        project: Uuid,
        contract_hash: [u8; 32],
        scope: LegacyVmPlacementScope,
        vm_id: VmId,
        producer: LegacyVmPlacementProducer,
        consumption: Option<LegacyVmPlacementConsumption>,
    ) -> Result<Self, LegacyVmPlacementError> {
        let pins = [
            command.run_id.as_uuid(),
            command.command_id.as_uuid(),
            command.instance_id.as_uuid(),
            command.instance_revision_id.as_uuid(),
            command.release_id.as_uuid(),
            command.release_agent_id.as_uuid(),
            project,
        ];
        if pins.iter().any(Uuid::is_nil)
            || command
                .attachment_id
                .is_some_and(|id| id.as_uuid().is_nil())
        {
            return Err(LegacyVmPlacementError::Pins);
        }
        if vm_id.0 != command.run_id.to_string() {
            return Err(LegacyVmPlacementError::VmIdentity);
        }
        let normal = command.kind == RunKind::Normal && command.attachment_id.is_some();
        let update = command.kind == RunKind::Update && command.attachment_id.is_none();
        if !matches!(
            (producer, normal, update),
            (
                LegacyVmPlacementProducer::NormalRequest | LegacyVmPlacementProducer::Mailbox,
                true,
                false
            ) | (LegacyVmPlacementProducer::UpdateHook, false, true)
        ) {
            return Err(LegacyVmPlacementError::Producer);
        }
        Ok(Self {
            command,
            project,
            contract_hash,
            scope,
            vm_id,
            producer,
            consumption,
        })
    }

    /// Returns all frozen command pins.
    #[must_use]
    pub const fn command(&self) -> &StartRun {
        &self.command
    }
    /// Returns the exact owning project UUID; this metadata pin grants no authority.
    #[must_use]
    pub const fn project(&self) -> Uuid {
        self.project
    }
    /// Returns the frozen released contract hash.
    #[must_use]
    pub const fn contract_hash(&self) -> &[u8; 32] {
        &self.contract_hash
    }
    /// Returns the exact configured provider scope.
    #[must_use]
    pub const fn scope(&self) -> &LegacyVmPlacementScope {
        &self.scope
    }
    /// Returns the recorded physical identity, including before Run VM binding.
    #[must_use]
    pub const fn vm_id(&self) -> &VmId {
        &self.vm_id
    }
    /// Returns the recorded birth producer.
    #[must_use]
    pub const fn producer(&self) -> LegacyVmPlacementProducer {
        self.producer
    }
    /// Returns permanent consumption, or unconsumed eligibility.
    #[must_use]
    pub const fn consumption(&self) -> Option<LegacyVmPlacementConsumption> {
        self.consumption
    }
}

/// Invalid shape; database provenance and live authorization remain separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LegacyVmPlacementError {
    /// Production ownership must use a canonical persistent owner UUID.
    #[error("Legacy VM namespace is not a canonical non-nil UUID")]
    Namespace,
    /// Frozen identity cannot contain nil resource IDs.
    #[error("Legacy placement contains nil pins")]
    Pins,
    /// Fresh managed Run VM identity is pinned to its Run UUID.
    #[error("Legacy planned VM differs from Run UUID")]
    VmIdentity,
    /// Producer kind must agree with the command's purpose and attachment.
    #[error("Legacy birth producer differs from command")]
    Producer,
}

/// Safe explanation for a Run excluded from managed Legacy recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyVmPlacementHoldReason {
    /// No positive fresh execution profile was recorded.
    HistoricalUnproven,
    /// Positive profile history has no prospective placement birth evidence.
    MissingBirth,
    /// Recorded ownership belongs to another provider namespace or host.
    OwnerMismatch,
    /// Stored pins or immutable producer correlation are contradictory.
    CorrelationMismatch,
}

/// Identifier-only held history; no provider paths or source payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyVmPlacementHold {
    /// Run retaining its resources and requiring explicit resolution.
    pub run_id: RunId,
    /// Closed reason for the hold.
    pub reason: LegacyVmPlacementHoldReason,
}

/// Read-only scoped inspection, not permission to destroy or release resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyVmPlacementInventory {
    /// Exact owner placements, including eligible, VM-NULL and terminal rows.
    pub placements: Vec<LegacyVmPlacement>,
    /// Unknown and known foreign evidence remains visible as held identifiers.
    pub held: Vec<LegacyVmPlacementHold>,
}

#[cfg(test)]
#[path = "legacy_vm_placement/tests.rs"]
mod tests;
