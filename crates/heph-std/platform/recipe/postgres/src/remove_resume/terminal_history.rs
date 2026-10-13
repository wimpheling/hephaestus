//! Retained configured pools for native143 historical observations only.
#[path = "terminal_history/actor.rs"]
mod actor;
#[path = "terminal_history/applied.rs"]
pub(super) mod applied;
#[path = "terminal_history/closure.rs"]
mod closure;
#[path = "terminal_history/encoding.rs"]
mod encoding;
#[path = "terminal_history/original.rs"]
mod original;
#[path = "terminal_history/worker.rs"]
mod worker;

use crate::PostgresDeploymentRepository;
use recipe_application::{
    DeploymentError, DeploymentExecutionProfile, PlatformPolicyObservation,
    PreparedInstanceTerminalHistoryRepository, RemoveResumeConfiguration,
};
use release_service::InstanceExecutionConfiguration;
use sqlx::{PgPool, Postgres, Transaction};

struct Reader {
    actor: PgPool,
    worker: PgPool,
    configuration: InstanceExecutionConfiguration,
    remove: RemoveResumeConfiguration,
}
impl PostgresDeploymentRepository {
    /// Retains this repository's actual actor pool and a dedicated worker pool.
    ///
    /// Trusted full configuration must come from the retained owning composition.
    /// Construction supplies no readiness, admission, physical or historical proof.
    ///
    /// # Errors
    /// Denies absent/mismatched Remove configuration or non-named execution profile.
    pub fn terminal_history(
        &self,
        worker: PgPool,
        configuration: InstanceExecutionConfiguration,
    ) -> Result<impl PreparedInstanceTerminalHistoryRepository, DeploymentError> {
        let remove = self
            .remove_resume
            .clone()
            .ok_or(DeploymentError::InvalidAction)?;
        let platform = PlatformPolicyObservation::new(
            configuration.platform_policy().clone(),
            configuration.platform_version().to_owned(),
        )?;
        if remove.profile() != DeploymentExecutionProfile::RuntimeNamedV1
            || remove.namespace() != configuration.provider_scope().namespace()
            || remove.host() != configuration.provider_scope().host_id()
            || remove.platform() != &platform
        {
            return Err(DeploymentError::IntentMismatch);
        }
        Ok(Reader {
            actor: self.pool.clone(),
            worker,
            configuration,
            remove,
        })
    }
}
async fn role(tx: &mut Transaction<'_, Postgres>, expected: &str) -> Result<(), DeploymentError> {
    let actual: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut **tx)
        .await
        .map_err(crate::repository_error)?;
    if actual != expected {
        return Err(DeploymentError::AuthorizationDenied);
    }
    sqlx::query("SET LOCAL search_path=pg_catalog,public,pg_temp")
        .execute(&mut **tx)
        .await
        .map_err(crate::repository_error)?;
    Ok(())
}
fn storage(error: sqlx::Error) -> DeploymentError {
    super::decoding::storage(error)
}
