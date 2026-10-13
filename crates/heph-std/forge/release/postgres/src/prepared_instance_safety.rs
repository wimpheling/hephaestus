//! Read-only qualified instance observation; no closure or runtime effects.
mod closure;
mod codec;
mod query;

pub use closure::{audit_all, claim, publication};
pub use codec::{Row as SafetyRow, observation as safety_observation};
pub use query::role;

use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentOperation, PlatformPolicyObservation,
    PreparedInstanceSafetyObservation, PreparedInstanceSafetyRepository,
    PreparedInstanceSafetyTarget,
};
use sqlx::PgPool;
use vm_trait::VmProviderOwnerScope;

/// Explicit backend adapter for original creation and complete runtime evidence.
///
/// Construction opts into the private readonly facade. It does not activate
/// instances, import under an old actor, stop VMs or record a completion. The
/// scope must come from the actual provider retained by owning composition.
#[derive(Clone)]
pub struct PostgresPreparedInstanceSafetyRepository {
    actor: PgPool,
    worker: PgPool,
    namespace: String,
    host: String,
    platform: PlatformPolicyObservation,
    remove_resume: Option<recipe_application::RemoveResumeConfiguration>,
}
impl PostgresPreparedInstanceSafetyRepository {
    /// Configures separate actor/worker pools and actual provider comparison pins.
    #[must_use]
    pub fn new(
        actor: PgPool,
        worker: PgPool,
        scope: &VmProviderOwnerScope,
        platform: PlatformPolicyObservation,
    ) -> Self {
        Self {
            actor,
            worker,
            namespace: scope.namespace().to_owned(),
            host: scope.host_id().to_owned(),
            platform,
            remove_resume: None,
        }
    }
    /// Enables read-only original closure lookup for an explicitly matched backend.
    ///
    /// Scope comparison comes from this adapter's actual provider configuration;
    /// enabling this API creates no admission, credentials or runtime effects.
    ///
    /// # Errors
    /// Rejects changed provider labels, platform or preparation profile.
    pub fn with_remove_resume(
        mut self,
        config: recipe_application::RemoveResumeConfiguration,
    ) -> Result<Self, DeploymentError> {
        if config.namespace() != self.namespace
            || config.host() != self.host
            || config.platform() != &self.platform
            || config.profile() != recipe_application::DeploymentExecutionProfile::RuntimeNamedV1
        {
            return Err(DeploymentError::IntentMismatch);
        }
        self.remove_resume = Some(config);
        Ok(self)
    }
    fn configured(&self, target: &PreparedInstanceSafetyTarget) -> Result<(), DeploymentError> {
        if target.provider_namespace() != self.namespace
            || target.host_id() != self.host
            || target.prepared().source().platform() != &self.platform
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(())
    }
}
#[async_trait]
impl PreparedInstanceSafetyRepository for PostgresPreparedInstanceSafetyRepository {
    async fn load_original_target(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        intent: &recipe_application::DeploymentIntent,
        resource: &capability_domain::CapabilitySlotKey,
    ) -> Result<PreparedInstanceSafetyTarget, DeploymentError> {
        closure::original(self, identity, command, intent, resource).await
    }
    async fn inspect_for_remove(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        target: &PreparedInstanceSafetyTarget,
    ) -> Result<PreparedInstanceSafetyObservation, DeploymentError> {
        command.validate(identity, DeploymentOperation::Remove)?;
        self.configured(target)?;
        let mut tx = self
            .actor
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await
            .map_err(|error| query::storage(&error))?;
        query::role(&mut tx, "hephaestus_app").await?;
        // Transaction-local provenance from the genuine authenticated request;
        // historical installer data is never converted into an identity.
        sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),set_config('hephaestus.request_id',$2,true),set_config('hephaestus.occurrence_id',$3,true)")
            .bind(identity.user_id.to_string()).bind(identity.request_id.to_string()).bind(identity.idempotency_id.to_string())
            .execute(&mut *tx).await.map_err(|error| query::storage(&error))?;
        let row = query::actor(&mut tx, command, target).await?;
        let observation = codec::observation(row, target)?;
        tx.commit().await.map_err(|error| query::storage(&error))?;
        Ok(observation)
    }
    async fn observe_safety(
        &self,
        target: &PreparedInstanceSafetyTarget,
    ) -> Result<PreparedInstanceSafetyObservation, DeploymentError> {
        self.configured(target)?;
        let mut tx = self
            .worker
            .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .await
            .map_err(|error| query::storage(&error))?;
        query::role(&mut tx, "hephaestus_worker").await?;
        let row = query::worker(&mut tx, target).await?;
        let observation = codec::observation(row, target)?;
        tx.commit().await.map_err(|error| query::storage(&error))?;
        Ok(observation)
    }
}
