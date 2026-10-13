//! Worker consumption of exact preexisting115 admission; no provider IO.

mod hydration;
mod queries;
pub mod reservations;
pub mod reservations119;

use release_service::InstanceExecutionConfiguration;
use run_domain::{FreshRunExecutionProfile, RunCleanupHostId, RunKind, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError};
use vm_trait::VmId;

use crate::{PgRunRepository, errors::storage};

impl PgRunRepository {
    pub(crate) fn invocation_configuration(
        &self,
    ) -> Result<&InstanceExecutionConfiguration, RepositoryError> {
        let configuration =
            self.invocation_configuration
                .as_ref()
                .ok_or(RepositoryError::InvalidData(
                    "qualified Invocation configuration is absent",
                ))?;
        let scope = self.vm_scope.as_ref().ok_or(RepositoryError::InvalidData(
            "Invocation requires actual configured provider owner",
        ))?;
        let owner_matches = (scope.provider_namespace(), scope.host_id())
            == (
                configuration.provider_scope().namespace(),
                configuration.provider_scope().host_id(),
            );
        if self.execution_profile != Some(FreshRunExecutionProfile::OwnedCanonical)
            || self.legacy_vm_scope.is_some()
            || !owner_matches
        {
            return Err(RepositoryError::InvalidData(
                "Invocation composition differs",
            ));
        }
        Ok(configuration)
    }

    pub(crate) async fn consume_invocation(
        &self,
        command: &StartRun,
        scope: &RunCleanupHostId,
        vm: &VmId,
    ) -> Result<CreateRunResult, RepositoryError> {
        let configuration = self.invocation_configuration()?;
        let exact_scope = self.vm_scope.as_ref() == Some(scope);
        let exact_vm = vm.0 == command.run_id.to_string();
        if command.kind != RunKind::Invocation
            || command.attachment_id.is_some()
            || !exact_scope
            || !exact_vm
        {
            return Err(RepositoryError::InvalidData(
                "exact Invocation Start/provider required",
            ));
        }
        let config = serde_json::to_vec(&configuration.canonical_json()).map_err(storage)?;
        let payload = serde_json::to_value(command).map_err(storage)?;
        let mut tx = self.profile_inventory_transaction().await?;
        if self.invocation_lease_runtime {
            reservations119::acquire(&mut tx, command.run_id).await?;
        } else {
            reservations::acquire(&mut tx, command.run_id).await?;
        }
        let row = queries::consume(
            &mut tx,
            command,
            scope,
            vm,
            payload,
            config,
            self.invocation_lease_runtime,
        )
        .await?;
        let run = row.into_checked_invocation()?;
        if !crate::model::matches_start_command(&run, command) {
            return Err(RepositoryError::InvalidData(
                "consumed Invocation differs from Start",
            ));
        }
        tx.commit().await.map_err(storage)?;
        Ok(CreateRunResult {
            run,
            created: false,
        })
    }
}
