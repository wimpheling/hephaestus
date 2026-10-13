//! Service construction and update lifecycle accessors.

use super::{
    AgentUpdateId, AuthenticatedIdentity, ObjectRef, ObjectType, Permission, PgPool,
    PostgresMelangeAuthorizer, ReleaseService, ReleaseServiceError, begin_actor_transaction,
};
use runtime_types::RunId;
use std::sync::Arc;
use uuid::Uuid;

impl ReleaseService {
    /// Creates the service.
    #[must_use]
    pub const fn new(pool: PgPool, authorizer: Arc<PostgresMelangeAuthorizer>) -> Self {
        Self {
            pool,
            authorizer,
            execution_profile: None,
            legacy_vm_scope: None,
        }
    }

    /// Selects explicit fresh update-hook producer SQL for the reviewed schema.
    ///
    /// The default constructor emits public107 SQL even if newer tables exist.
    /// Owned selection requires fresh actor-authorized planning admission;
    /// replay retains its original profile and cannot be promoted.
    #[must_use]
    pub const fn with_execution_profile(
        mut self,
        profile: run_domain::FreshRunExecutionProfile,
    ) -> Self {
        self.execution_profile = Some(profile);
        self
    }

    /// Reads the hook run durably admitted for an update after a concurrent
    /// reconciler wins the admission race.
    ///
    /// # Errors
    ///
    /// Fails when the update is unavailable, the actor is unauthorized, or
    /// `PostgreSQL` cannot read the lifecycle row.
    pub async fn current_update_hook_run(
        &self,
        identity: &AuthenticatedIdentity,
        update_id: AgentUpdateId,
    ) -> Result<Option<RunId>, ReleaseServiceError> {
        let mut tx = begin_actor_transaction(&self.pool, identity).await?;
        let instance_id: Uuid =
            sqlx::query_scalar("SELECT instance_id FROM agent_updates WHERE id = $1 FOR UPDATE")
                .bind(update_id.as_uuid())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(ReleaseServiceError::Unavailable)?;
        self.require(
            &mut tx,
            identity,
            Permission::CanUpdate,
            ObjectRef::new(ObjectType::AgentInstance, instance_id),
        )
        .await?;
        let hook_run_id: Option<Uuid> =
            sqlx::query_scalar("SELECT hook_run_id FROM agent_updates WHERE id = $1")
                .bind(update_id.as_uuid())
                .fetch_one(&mut *tx)
                .await?;
        tx.commit().await?;
        Ok(hook_run_id.map(RunId::from_uuid))
    }
}
