use crate::{
    EphemeralSecretConfig, RawSecretFile, SecretDispatchInput, SecretMountMetadata,
    SecretMountProvider, SecretRuntimeError,
};
use async_trait::async_trait;
use forge_domain::{CommitSha, GitRef};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use run_domain::{Run, RunKind};
use run_orchestrator::{PreparedRunSecrets, RunSecretError, RunSecretManager};
use runtime_types::RunId;
use secret_application::{ResolveRunSecrets, SecretDispatchResolver, SecretRuntimeResolver};
use secret_domain::{DeliveryMode, ExecutionPhase, SecretCommandKey, SecretRuntimeSessionId};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

/// Live-dispatch and ephemeral-mount integration for the run orchestrator.
pub struct SecretMountManager<M, D, R> {
    metadata: M,
    dispatch: D,
    runtime: R,
    provider: Arc<dyn SecretMountProvider>,
    config: EphemeralSecretConfig,
}

impl<M, D, R> SecretMountManager<M, D, R>
where
    M: SecretMountMetadata + 'static,
    D: SecretDispatchResolver,
    R: SecretRuntimeResolver,
{
    /// Validates configuration and creates a manager from separate services.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when the provider rejects the configuration.
    pub fn initialize(
        metadata: M,
        dispatch: D,
        runtime: R,
        provider: Arc<dyn SecretMountProvider>,
        config: EphemeralSecretConfig,
    ) -> Result<Self, RunSecretError> {
        provider
            .validate_config(&config)
            .map_err(secret_runtime_error)?;
        Ok(Self {
            metadata,
            dispatch,
            runtime,
            provider,
            config,
        })
    }

    async fn dispatch_input(
        &self,
        run: &Run,
    ) -> Result<Option<SecretDispatchInput>, RunSecretError> {
        let row = self
            .metadata
            .dispatch_input(run)
            .await?
            .ok_or_else(|| secret_error("exact secret dispatch provenance is unavailable"))?;
        let bindings: Vec<Uuid> =
            serde_json::from_value(row.secret_bindings.clone()).map_err(secret_serialization)?;
        if bindings.is_empty() {
            Ok(None)
        } else {
            Ok(Some(row))
        }
    }
}

#[async_trait]
impl<M, D, R> RunSecretManager for SecretMountManager<M, D, R>
where
    M: SecretMountMetadata + 'static,
    D: SecretDispatchResolver + 'static,
    R: SecretRuntimeResolver + 'static,
{
    async fn prepare(&self, run: &Run) -> Result<PreparedRunSecrets, RunSecretError> {
        let Some(input) = self.dispatch_input(run).await? else {
            return Ok(PreparedRunSecrets::default());
        };
        let actor_id = input
            .actor_id
            .ok_or_else(|| secret_error("secret-bearing run has no authenticated actor"))?;
        let request_id = input.request_id.unwrap_or_else(Uuid::new_v4);
        let identity = AuthenticatedIdentity::new(
            UserId::from_uuid(actor_id),
            "internal-run-dispatch",
            actor_id.to_string(),
            serde_json::json!({}),
            RequestId::from_uuid(request_id),
        );
        let target_ref = input
            .git_ref
            .map(GitRef::parse)
            .transpose()
            .map_err(|_| secret_error("secret-bearing run target ref is invalid"))?;
        let target_commit = input
            .commit_sha
            .map(CommitSha::parse)
            .transpose()
            .map_err(|_| secret_error("secret-bearing run target commit is invalid"))?;
        let authority = self
            .dispatch
            .resolve_for_dispatch(
                &identity,
                ResolveRunSecrets {
                    command_key: SecretCommandKey::derive(
                        "dispatch",
                        &[run.id.as_uuid().as_bytes()],
                    ),
                    session_id: SecretRuntimeSessionId::new(),
                    run_id: run.id,
                    instance_id: run.instance_id,
                    instance_revision_id: run.instance_revision_id,
                    attachment_id: run.attachment_id,
                    target_ref,
                    target_commit,
                    phase: match run.kind {
                        RunKind::Normal => ExecutionPhase::Normal,
                        RunKind::Update => ExecutionPhase::Update,
                    },
                    expires_at: time::OffsetDateTime::now_utc() + Duration::from_secs(600),
                },
            )
            .await
            .map_err(secret_service_error)?;
        let mut raw = Vec::new();
        for lease in &authority.leases {
            if lease.mode == DeliveryMode::Raw {
                let resolved = self
                    .runtime
                    .receive_raw(&authority.credential, run.id, lease.slot.clone())
                    .await
                    .map_err(secret_service_error)?;
                raw.push(RawSecretFile {
                    slot: resolved.slot,
                    value: resolved.value,
                });
            }
        }
        let mount = self
            .provider
            .materialize(&self.config, run.id, raw, Some(&authority.credential))
            .map_err(secret_runtime_error)?;
        if let Err(error) = self
            .metadata
            .persist_mount(run.id, mount.opaque_directory)
            .await
        {
            self.provider
                .discard_materialized(&self.config, mount.opaque_directory)
                .map_err(secret_runtime_error)?;
            return Err(error);
        }
        Ok(PreparedRunSecrets {
            mounts: vec![mount.vm_mount],
        })
    }

    async fn reauthorize(&self, run: &Run) -> Result<(), RunSecretError> {
        if self.dispatch_input(run).await?.is_none() {
            return Ok(());
        }
        if self.metadata.authorized(run).await? {
            Ok(())
        } else {
            Err(secret_error("live secret authority was revoked"))
        }
    }

    async fn destroy_after_guest(&self, run_id: RunId) -> Result<(), RunSecretError> {
        let Some(directory) = self.metadata.materialized_directory(run_id).await? else {
            return Ok(());
        };
        self.provider
            .destroy_confirmed(&self.config, directory)
            .map_err(secret_runtime_error)?;
        self.metadata.mark_destroyed(run_id).await
    }

    async fn recover(&self) -> Result<usize, RunSecretError> {
        let names = self.metadata.live_directories().await?;
        let removed = self
            .provider
            .reconcile_orphans(&self.config, &names)
            .map_err(secret_runtime_error)?;
        self.metadata.mark_cleaned_mounts_destroyed().await?;
        Ok(removed)
    }
}

fn secret_error(message: impl Into<String>) -> RunSecretError {
    RunSecretError::redacted(message)
}

fn secret_service_error(_error: secret_application::SecretServiceError) -> RunSecretError {
    secret_error("live secret dispatch failed")
}

fn secret_runtime_error(_error: SecretRuntimeError) -> RunSecretError {
    secret_error("ephemeral secret filesystem operation failed")
}

fn secret_serialization(_error: serde_json::Error) -> RunSecretError {
    secret_error("stored secret binding provenance is invalid")
}
