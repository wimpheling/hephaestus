//! Explicit qualified activation, separate from legacy Run producer selection.

mod activation;
mod configuration;
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
        }
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
        _identity: &AuthenticatedIdentity,
        _command: InvokeInstance,
    ) -> Result<InstanceInvocationAdmission, InstanceExecutionError> {
        // Invocation SQL/kind/dispatch is a separate coherent implementation.
        Err(InstanceExecutionError::Unsupported)
    }
}
