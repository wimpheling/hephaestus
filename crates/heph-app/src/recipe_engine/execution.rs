use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AdmissionDisposition, DeploymentAdmission, DeploymentExecutionPreparationRepository,
    DeploymentLifecycle, PlannedResourceIdentity, ResourceOwnership,
};

use super::{PreparedRecipeApplication, RecipeApplicationError, RecipeInstallation, validation};

impl PreparedRecipeApplication {
    pub(super) async fn execute(
        &self,
        identity: &AuthenticatedIdentity,
        admission: DeploymentAdmission,
    ) -> Result<RecipeInstallation, RecipeApplicationError> {
        let original = admission.receipt.command;
        let intent = &admission.snapshot.intent;
        validation::supported(intent, self.retained_volumes.is_some())?;
        let frozen = self
            .preparation
            .prepare_execution(identity, original, intent.id())
            .await?
            .preparation;
        validation::preparation(intent, original, &frozen)?;
        for name in intent.execution_order() {
            let before = self.repository.inspect(identity, intent.id()).await?;
            validation::installing(&before, intent)?;
            let recorded = before
                .resources
                .get(name)
                .ok_or(recipe_application::DeploymentError::IntentMismatch)?;
            let target = intent
                .resources()
                .get(name)
                .ok_or(recipe_application::DeploymentError::IntentMismatch)?;
            let progress = if target.ownership() == ResourceOwnership::External {
                // External Ready is comparison data: only the owning adapter's
                // exact original claim and paired receipt can prove replay.
                self.verify_external(identity, original, intent, &frozen, name)
                    .await?
            } else {
                // Fresh whole preparation and exact authorized intent replay
                // precede the existing protected owned106 Ready shortcut.
                if validation::ready(recorded).is_ok() {
                    continue;
                }
                match target.identity() {
                    PlannedResourceIdentity::Volume { .. } => self
                        .volumes
                        .create_prepared_volume(identity, original, intent.id(), name)
                        .await
                        .map_err(|error| RecipeApplicationError::Volume(Box::new(error)))?,
                    PlannedResourceIdentity::Instance { .. } => self
                        .instances
                        .import_prepared_instance(identity, original, intent.id(), name)
                        .await
                        .map_err(|error| RecipeApplicationError::Instance(Box::new(error)))?,
                }
            };
            validation::ready(&progress)?;
            let after = self.repository.inspect(identity, intent.id()).await?;
            validation::installing(&after, intent)?;
            let stored = after
                .resources
                .get(name)
                .ok_or(recipe_application::DeploymentError::IntentMismatch)?;
            validation::ready(stored)?;
            if stored != &progress {
                return Err(recipe_application::DeploymentError::StaleClaim.into());
            }
        }
        let current = self.repository.inspect(identity, intent.id()).await?;
        validation::installing(&current, intent)?;
        let completion = if admission.disposition == AdmissionDisposition::Replay {
            if current.lifecycle != DeploymentLifecycle::Installed {
                return Err(recipe_application::DeploymentError::IntentMismatch.into());
            }
            None
        } else {
            Some(
                self.repository
                    .finish_command(identity, original, intent.id(), current.version)
                    .await?,
            )
        };
        let activations = self.activate(identity, original, intent, &frozen).await;
        let ledger = self.repository.inspect(identity, intent.id()).await?;
        validation::installing(&ledger, intent)?;
        Ok(RecipeInstallation {
            admission,
            completion,
            ledger,
            activations,
        })
    }
}
