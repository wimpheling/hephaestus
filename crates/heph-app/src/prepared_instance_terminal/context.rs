//! Concrete ports derived only from the SAME original owning controller.
use super::PreparedInstanceTerminalError;
use crate::ManagedInvocationController;
use recipe_application::{
    DeploymentExecutionProfile, PlatformPolicyObservation,
    PreparedInstanceTerminalHistoryRepository, RemoveResumeConfiguration,
};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder,
    PostgresExecutionPreparationRepository,
};
use release_postgres::{
    PostgresPreparedInstanceSafetyRepository, PostgresPreparedInstanceTerminalRepository,
};
use std::sync::Arc;

pub(super) struct Context {
    pub controller: Arc<ManagedInvocationController>,
    pub deployment: PostgresDeploymentRepository,
    pub preparation: PostgresExecutionPreparationRepository,
    pub safety: PostgresPreparedInstanceSafetyRepository,
    pub terminal: PostgresPreparedInstanceTerminalRepository,
    pub history: Arc<dyn PreparedInstanceTerminalHistoryRepository>,
    pub recorder: PostgresEffectVerificationRecorder,
    pub remove: RemoveResumeConfiguration,
}
impl Context {
    pub fn new(
        controller: Arc<ManagedInvocationController>,
    ) -> Result<Self, PreparedInstanceTerminalError> {
        controller.validate_owners()?;
        let inputs = controller.drain_inputs();
        let cfg = &inputs.configuration;
        let scope = cfg.provider_scope();
        let platform = PlatformPolicyObservation::new(
            cfg.platform_policy().clone(),
            cfg.platform_version().to_owned(),
        )?;
        let remove = RemoveResumeConfiguration::new(
            scope.namespace().to_owned(),
            scope.host_id().to_owned(),
            platform.clone(),
            DeploymentExecutionProfile::RuntimeNamedV1,
        )?;
        let deployment = PostgresDeploymentRepository::new(inputs.pools.application.clone())
            .with_remove_resume(remove.clone());
        let history =
            Arc::new(deployment.terminal_history(inputs.pools.worker.clone(), cfg.clone())?);
        let preparation = PostgresExecutionPreparationRepository::new(
            inputs.pools.application.clone(),
            platform.clone(),
            DeploymentExecutionProfile::RuntimeNamedV1,
        )
        .with_remove_resume(remove.clone())?;
        let safety = PostgresPreparedInstanceSafetyRepository::new(
            inputs.pools.application.clone(),
            inputs.pools.worker.clone(),
            scope,
            platform,
        )
        .with_remove_resume(remove.clone())?;
        let terminal = PostgresPreparedInstanceTerminalRepository::new(
            inputs.pools.application.clone(),
            inputs.pools.worker.clone(),
            cfg.clone(),
        );
        let recorder = PostgresEffectVerificationRecorder::new(inputs.pools.worker.clone());
        Ok(Self {
            controller,
            deployment,
            preparation,
            safety,
            terminal,
            history,
            recorder,
            remove,
        })
    }
}
