//! Current-manager Ready retention; original Install and default adapters remain separate.
mod closure;
mod context;
mod reopening;
mod resume;
mod retained_external;
mod transition;
use crate::{
    PostgresDeploymentRepository, PostgresExecutionPreparationRepository, repository_error,
};
use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentError, OwnedReadyRetentionPlan, OwnedReadyRetentionRepository,
    ReopenRetainedOwnedVolume, ResourceProgress, RetainOwnedReady, RetainedExternalCompletion,
    RetainedExternalVerificationPlan,
};
use sqlx::PgPool;
use uuid::Uuid;

/// Explicit Ready successor adapter with trusted actual VM comparison pins.
///
/// No constructor bootstraps roots or enables runtime dispatch. Configure the opaque namespace
/// and host from the actual provider's checked scope, never disk namespace,
/// payload, historical row or current process. They are comparison data only.
#[derive(Clone)]
pub struct PostgresOwnedReadyRetentionRepository {
    repository: PostgresDeploymentRepository,
    preparation: PostgresExecutionPreparationRepository,
    namespace: String,
    host: String,
}
impl PostgresOwnedReadyRetentionRepository {
    /// Configures explicit backend scope and current whole-preparation validation.
    ///
    /// # Errors
    /// Invalid opaque scope pins deny composition; default repository paths are unchanged.
    pub fn new(
        pool: PgPool,
        preparation: PostgresExecutionPreparationRepository,
        namespace: String,
        host: String,
    ) -> Result<Self, DeploymentError> {
        if namespace.is_empty()
            || namespace.len() > 128
            || matches!(namespace.as_str(), "." | "..")
            || !namespace.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            })
            || host.is_empty()
            || host.len() > 128
            || matches!(host.as_str(), "." | "..")
            || !host.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            })
        {
            return Err(DeploymentError::InvalidIdentifier);
        }
        Ok(Self {
            repository: PostgresDeploymentRepository::new(pool),
            preparation,
            namespace,
            host,
        })
    }
}
#[async_trait]
impl OwnedReadyRetentionRepository for PostgresOwnedReadyRetentionRepository {
    async fn resume_owned_ready_remove(
        &self,
        identity: &AuthenticatedIdentity,
        remove: &recipe_application::RemoveDeployment,
        target: &recipe_application::EffectClaim,
    ) -> Result<recipe_application::OwnedReadyRemoveResume, DeploymentError> {
        resume::admit(self, identity, remove, target).await
    }

    async fn begin_owned_ready_retention(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainOwnedReady,
    ) -> Result<OwnedReadyRetentionPlan, DeploymentError> {
        let mut context = context::load(self, identity, request).await?;
        let preparation = context::prepared(&mut context, &request.target).await?;
        let closure = closure::ensure(self, &mut context, identity, request).await?;
        let completed = transition::completed(&mut context, request, closure).await?;
        if completed.is_none() {
            context::current(&mut context, request).await?;
        }
        context.tx.commit().await.map_err(repository_error)?;
        Ok(OwnedReadyRetentionPlan {
            preparation,
            closure_id: closure,
            vm_provider_namespace: self.namespace.clone(),
            vm_host_id: self.host.clone(),
            completed,
        })
    }
    async fn retain_owned_ready(
        &self,
        identity: &AuthenticatedIdentity,
        request: &RetainOwnedReady,
        fact: Uuid,
    ) -> Result<ResourceProgress, DeploymentError> {
        let mut context = context::load(self, identity, request).await?;
        context::prepared(&mut context, &request.target).await?;
        let closure = closure::ensure(self, &mut context, identity, request).await?;
        if let Some(progress) = transition::completed(&mut context, request, closure).await? {
            transition::exact_fact(&mut context.tx, request, fact, closure).await?;
            context.tx.commit().await.map_err(repository_error)?;
            return Ok(progress);
        }
        let before = context::current(&mut context, request).await?;
        let after =
            transition::commit(&mut context, identity, request, &before, closure, fact).await?;
        context.tx.commit().await.map_err(repository_error)?;
        after.progress()
    }
    async fn reopen_retained_owned_volume(
        &self,
        identity: &AuthenticatedIdentity,
        request: &ReopenRetainedOwnedVolume,
    ) -> Result<(), DeploymentError> {
        reopening::reopen(self, identity, request).await
    }
    async fn prepare_retained_external_verification(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &recipe_application::EffectClaim,
    ) -> Result<RetainedExternalVerificationPlan, DeploymentError> {
        retained_external::prepare(self, identity, claim).await
    }
    async fn complete_retained_external_verification(
        &self,
        identity: &AuthenticatedIdentity,
        plan: &RetainedExternalVerificationPlan,
        completion: RetainedExternalCompletion,
    ) -> Result<ResourceProgress, DeploymentError> {
        retained_external::complete(self, identity, plan, completion).await
    }
}
