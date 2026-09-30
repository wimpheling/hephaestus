use super::AttemptError;
use crate::ReleaseService;
use identity_domain::AuthenticatedIdentity;
use release_domain::UiInstallationOperation;
use release_service::{
    ActivateUiInstallation, DisableUiInstallation, InstallStaticUi, InstallStaticUiResult,
    InstallUi, InstallUiResult, RemoveUiInstallation, RollbackUiInstallation, UiInstallationError,
    UiInstallationGenerationResult, UiInstallationLifecycleResult,
};

impl ReleaseService {
    /// Installs one published UI and pins every declared gateway binding.
    ///
    /// The owner row is locked before authorization, replay lookup, or
    /// generation creation. Gateway rows and source parent rows are then
    /// resolved under the same transaction, so an invalid binding rolls back
    /// the complete installation receipt.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure for authorization, unsupported publication
    /// content, unavailable gateway bindings, idempotency conflict, an
    /// existing active installation, or unavailable persistence.
    pub async fn install_ui(
        &self,
        identity: &AuthenticatedIdentity,
        command: InstallUi,
    ) -> Result<InstallUiResult, UiInstallationError> {
        for attempt in 0..2 {
            match self.install_ui_once(identity, &command, false).await {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => return Ok(result),
            }
        }
        Err(UiInstallationError::Unavailable)
    }

    /// Installs one published static UI with no API or managed bindings.
    ///
    /// This compatibility wrapper delegates to the generalized installation
    /// transaction while retaining the original static zero-binding contract.
    ///
    /// # Errors
    ///
    /// Returns a redacted, transport-neutral failure for authorization,
    /// unsupported publication content, idempotency conflict, an existing
    /// active installation, or unavailable persistence.
    ///
    /// The owner row is locked before authorization, command replay, or
    /// installation lookup. A second bounded attempt handles a same-actor
    /// caller-key race on a different owner row by rereading the committed
    /// immutable ledger; it never retries a failed transaction in place.
    ///
    /// # Errors
    ///
    /// Returns a redacted, transport-neutral failure for authorization,
    /// unsupported publication content, idempotency conflict, an existing
    /// active installation, or unavailable persistence.
    pub async fn install_static_ui(
        &self,
        identity: &AuthenticatedIdentity,
        command: InstallStaticUi,
    ) -> Result<InstallStaticUiResult, UiInstallationError> {
        let command = InstallUi {
            caller_key: command.caller_key,
            target: command.target,
            release_id: command.release_id,
            ui_key: command.ui_key,
            expected_organization_id: None,
            acknowledge_repository_git_access: false,
        };
        for attempt in 0..2 {
            match self.install_ui_once(identity, &command, true).await {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => {
                    return Ok(InstallStaticUiResult {
                        installation_id: result.installation_id,
                        generation_id: result.generation_id,
                        state: result.state,
                        idempotency_id: result.idempotency_id,
                    });
                }
            }
        }
        Err(UiInstallationError::Unavailable)
    }

    /// Activates a new immutable generation for an installation.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure for unavailable persistence, denied current
    /// owner or source authority, stale generation CAS, invalid publication,
    /// terminal installation state, or changed replay input.
    pub async fn activate_ui_installation(
        &self,
        identity: &AuthenticatedIdentity,
        command: ActivateUiInstallation,
    ) -> Result<UiInstallationGenerationResult, UiInstallationError> {
        for attempt in 0..2 {
            match self
                .mutate_ui_generation(
                    identity,
                    command.installation_id,
                    command.caller_key.clone(),
                    command.expected_generation_id,
                    command.release_id,
                    command.ui_key.clone(),
                    UiInstallationOperation::Activate,
                )
                .await
            {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => return Ok(result),
            }
        }
        Err(UiInstallationError::Unavailable)
    }

    /// Rolls an installation back to a selected release as a new immutable
    /// generation; historical generations remain addressable.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure for unavailable persistence, denied current
    /// owner or source authority, stale generation CAS, invalid publication,
    /// terminal installation state, or changed replay input.
    pub async fn rollback_ui_installation(
        &self,
        identity: &AuthenticatedIdentity,
        command: RollbackUiInstallation,
    ) -> Result<UiInstallationGenerationResult, UiInstallationError> {
        for attempt in 0..2 {
            match self
                .mutate_ui_generation(
                    identity,
                    command.installation_id,
                    command.caller_key.clone(),
                    command.expected_generation_id,
                    command.release_id,
                    command.ui_key.clone(),
                    UiInstallationOperation::Rollback,
                )
                .await
            {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => return Ok(result),
            }
        }
        Err(UiInstallationError::Unavailable)
    }

    /// Disables an installation while retaining its current generation and
    /// immutable history. Source-release permissions are deliberately not
    /// consulted: current ownership authority controls this lifecycle change.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure for unavailable persistence, denied current
    /// owner authority, stale generation CAS, invalid lifecycle state, or a
    /// changed replay input.
    pub async fn disable_ui_installation(
        &self,
        identity: &AuthenticatedIdentity,
        command: DisableUiInstallation,
    ) -> Result<UiInstallationLifecycleResult, UiInstallationError> {
        for attempt in 0..2 {
            match self
                .mutate_ui_installation(
                    identity,
                    command.installation_id,
                    command.caller_key.clone(),
                    command.expected_generation_id,
                    UiInstallationOperation::Disable,
                )
                .await
            {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => return Ok(result),
            }
        }
        Err(UiInstallationError::Unavailable)
    }

    /// Terminally removes an installation while retaining its current
    /// generation and immutable history. The owner/key can then be reused by
    /// a new installation identity.
    ///
    /// # Errors
    ///
    /// Returns a redacted failure for unavailable persistence, denied current
    /// owner authority, stale generation CAS, invalid lifecycle state, or a
    /// changed replay input.
    pub async fn remove_ui_installation(
        &self,
        identity: &AuthenticatedIdentity,
        command: RemoveUiInstallation,
    ) -> Result<UiInstallationLifecycleResult, UiInstallationError> {
        for attempt in 0..2 {
            match self
                .mutate_ui_installation(
                    identity,
                    command.installation_id,
                    command.caller_key.clone(),
                    command.expected_generation_id,
                    UiInstallationOperation::Remove,
                )
                .await
            {
                Err(AttemptError::RetryLedgerRace) if attempt == 0 => {}
                Err(AttemptError::RetryLedgerRace) => return Err(UiInstallationError::Unavailable),
                Err(AttemptError::Public(error)) => return Err(error),
                Ok(result) => return Ok(result),
            }
        }
        Err(UiInstallationError::Unavailable)
    }
}
