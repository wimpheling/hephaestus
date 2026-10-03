use crate::{
    DeploymentError, DeploymentOperation, EffectClaim, PlannedResourceIdentity,
    PreparedInstanceImport, ResourceAction,
};

/// Structurally checked original creation and actual-provider comparison pins.
///
/// Construction is not a database observation or authorization decision. The
/// backend independently compares the full original claim and prepared input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInstanceSafetyTarget {
    original: EffectClaim,
    prepared: PreparedInstanceImport,
    namespace: String,
    host: String,
}
impl PreparedInstanceSafetyTarget {
    /// Checks original Create shape, complete prepared identity and bounded scope.
    ///
    /// Namespace is an opaque actual provider label, distinct from disk ownership.
    ///
    /// # Errors
    /// Rejects nil/changed original identity, non-Create/Install lineage, or labels
    /// outside the existing cleanup identifier bounds.
    pub fn new(
        original: EffectClaim,
        prepared: PreparedInstanceImport,
        namespace: String,
        host: String,
    ) -> Result<Self, DeploymentError> {
        let expected = PlannedResourceIdentity::Instance {
            id: prepared.instance_id(),
            revision_id: prepared.revision_id(),
        };
        if original.command.operation() != DeploymentOperation::Install
            || original.action != ResourceAction::Create
            || original.identity != expected
            || original.resource != *prepared.resource()
            || original.provenance.actor_id != original.command.actor_id()
            || original.resource_version == 0
            || original.generation == 0
            || original.provenance.request_id.as_uuid().is_nil()
            || prepared.instance_id().as_uuid().is_nil()
            || prepared.revision_id().as_uuid().is_nil()
            || prepared.project_id().as_uuid().is_nil()
            || prepared.operation_id().is_nil()
            || !valid_label(&namespace)
            || !valid_label(&host)
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Self {
            original,
            prepared,
            namespace,
            host,
        })
    }
    /// Returns the immutable original claim; never a reconstructed actor identity.
    #[must_use]
    pub const fn original(&self) -> &EffectClaim {
        &self.original
    }
    /// Returns the complete original import comparison input, including zero slots.
    #[must_use]
    pub const fn prepared(&self) -> &PreparedInstanceImport {
        &self.prepared
    }
    /// Returns the configured actual opaque provider namespace comparison pin.
    #[must_use]
    pub fn provider_namespace(&self) -> &str {
        &self.namespace
    }
    /// Returns the configured actual host comparison pin.
    #[must_use]
    pub fn host_id(&self) -> &str {
        &self.host
    }
}
fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !matches!(value, "." | "..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}
