//! Explicit qualified activation, separate from legacy Run producer selection.

mod activation;
mod configuration;
mod invocation;
mod invocation_reservation;
mod reservation;
mod rows;

pub use configuration::InstanceExecutionConfiguration;

use std::sync::Arc;

use async_trait::async_trait;
use authz_postgres::PostgresMelangeAuthorizer;
use identity_domain::AuthenticatedIdentity;
use release_service::{
    ActivateInstance, InstanceActivationAdmission, InstanceExecutionError,
    InstanceExecutionService, InstanceInvocationAdmission, InvokeInstance,
};
use sqlx::PgPool;

use crate::ReleaseService;

/// Configured activation adapter; legacy service producer configuration is separate.
///
/// Configuration is supplied by trusted provider composition. Ownership labels
/// are data, and this adapter neither performs IO nor proves physical readiness.
pub struct PostgresInstanceExecutionService {
    pool: PgPool,
    authorization: ReleaseService,
    configuration: InstanceExecutionConfiguration,
    qualified_invocation_admission: bool,
}

impl PostgresInstanceExecutionService {
    /// Constructs an explicitly configured qualified activation adapter.
    ///
    /// The caller must validate actual provider and root ownership before supplying
    /// this configuration. No client input or table-presence probe selects it.
    #[must_use]
    pub fn new(
        pool: PgPool,
        authorizer: Arc<PostgresMelangeAuthorizer>,
        configuration: InstanceExecutionConfiguration,
    ) -> Self {
        Self {
            authorization: ReleaseService::new(pool.clone(), authorizer),
            pool,
            configuration,
            qualified_invocation_admission: false,
        }
    }
    /// Selects the explicit115 admission adapter in trusted server composition.
    ///
    /// Requires the private115 schema and its qualified114 prerequisite.
    /// Default construction returns Unsupported before Invocation SQL. There is
    /// no schema probing or automatic enablement. Success records queued
    /// admission only; it does not enable runtime execution or create a VM.
    /// This grants no caller, producer, mount or runtime authority. The immutable
    /// request records qualification version115 separately from114 config bytes.
    #[must_use]
    pub const fn with_qualified_invocation_admission(mut self) -> Self {
        self.qualified_invocation_admission = true;
        self
    }
}

#[async_trait]
impl InstanceExecutionService for PostgresInstanceExecutionService {
    async fn activate_instance(
        &self,
        identity: &AuthenticatedIdentity,
        command: ActivateInstance,
    ) -> Result<InstanceActivationAdmission, InstanceExecutionError> {
        activation::activate(self, identity, command).await
    }

    async fn invoke_instance(
        &self,
        identity: &AuthenticatedIdentity,
        command: InvokeInstance,
    ) -> Result<InstanceInvocationAdmission, InstanceExecutionError> {
        if !self.qualified_invocation_admission {
            return Err(InstanceExecutionError::Unsupported);
        }
        invocation::invoke(self, identity, command).await
    }
}
