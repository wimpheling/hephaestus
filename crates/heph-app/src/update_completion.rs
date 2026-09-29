#[cfg(feature = "test-fixtures")]
use super::application;
use super::command_transport::completion_error;
use super::{
    Arc, CancellationToken, Duration, Mutex, OffsetDateTime, PgPool, ReleaseService,
    ReleaseServiceError, Run, RunCompletionError, RunCompletionObserver, RunId, RunKind, Uuid,
};
use async_trait::async_trait;
use control_plane_postgres::{
    is_update_hook_run, pending_update_admissions, recoverable_update_hook_run_ids,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::ReleaseCommandKey;
use release_service::BeginUpdateHook;
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;
pub struct UpdateRunCompletion {
    pub pool: PgPool,
    pub releases: Arc<ReleaseService>,
    pub admission_cursor: Mutex<Option<(OffsetDateTime, Uuid)>>,
}

impl UpdateRunCompletion {
    async fn next_pending_admissions(
        &self,
    ) -> Result<Vec<control_plane_postgres::PendingUpdateAdmission>, RunCompletionError> {
        let mut cursor = self.admission_cursor.lock().await;
        let admissions = pending_update_admissions(&self.pool, *cursor)
            .await
            .map_err(completion_error)?;
        if admissions.is_empty() {
            *cursor = None;
            return Ok(Vec::new());
        }
        *cursor = admissions
            .last()
            .map(|admission| (admission.created_at, admission.update_id));
        drop(cursor);
        Ok(admissions)
    }

    async fn resume_pending_admission(
        &self,
        admission: control_plane_postgres::PendingUpdateAdmission,
    ) -> Result<bool, RunCompletionError> {
        let identity = AuthenticatedIdentity::new(
            UserId::from_uuid(admission.actor_id),
            "hephaestus-update-recovery",
            "durable-update-recovery",
            serde_json::json!({}),
            RequestId::new(),
        );
        let generation = admission.generation.to_be_bytes();
        let hook_run_id =
            deterministic_update_hook_run_id(admission.update_id, admission.generation);
        let command_key = ReleaseCommandKey::derive(
            "begin_update_hook.recovery",
            &[admission.update_id.as_bytes(), &generation],
        );
        let result = self
            .releases
            .begin_update_hook(
                &identity,
                BeginUpdateHook {
                    command_key,
                    update_id: release_domain::AgentUpdateId::from_uuid(admission.update_id),
                    hook_run_id,
                },
            )
            .await;
        match result {
            Ok(()) => {
                #[cfg(feature = "test-fixtures")]
                application::commands::notify_reconciler_update_admission(admission.update_id);
                Ok(true)
            }
            Err(error) => {
                classify_pending_admission_error(admission.update_id, admission.actor_id, error)
            }
        }
    }

    async fn resume_pending_admissions(&self) -> Result<usize, RunCompletionError> {
        let admissions = self.next_pending_admissions().await?;
        let mut resumed = 0;
        for admission in admissions {
            if self.resume_pending_admission(admission).await? {
                resumed += 1;
            }
        }
        Ok(resumed)
    }

    async fn apply(&self, run: &Run) -> Result<bool, RunCompletionError> {
        if run.kind != RunKind::Update {
            return Ok(false);
        }
        let is_update_hook = is_update_hook_run(&self.pool, run.id.as_uuid())
            .await
            .map_err(completion_error)?;
        if !is_update_hook {
            return Ok(false);
        }
        self.releases
            .reconcile_update_run(run.id)
            .await
            .map_err(completion_error)?;
        Ok(true)
    }
}

fn classify_pending_admission_error(
    update_id: Uuid,
    actor_id: Uuid,
    error: ReleaseServiceError,
) -> Result<bool, RunCompletionError> {
    match admission_failure_kind(&error) {
        AdmissionFailureKind::DrainPending => Ok(log_drain_pending(update_id)),
        AdmissionFailureKind::AuthorizationDenied => {
            Ok(log_authorization_denied(update_id, actor_id))
        }
        AdmissionFailureKind::InvalidLifecycle => Ok(log_invalid_lifecycle(update_id)),
        AdmissionFailureKind::GenerationRace => Ok(log_generation_race(update_id)),
        AdmissionFailureKind::Other => Err(completion_error(error)),
    }
}

fn log_drain_pending(update_id: Uuid) -> bool {
    tracing::debug!(
        update_id = %update_id,
        "durable update remains fenced until normal work cleans up"
    );
    false
}

fn log_authorization_denied(update_id: Uuid, actor_id: Uuid) -> bool {
    tracing::error!(
        update_id = %update_id,
        actor_id = %actor_id,
        "durable update admission authorization was denied; leaving it fenced"
    );
    false
}

fn log_invalid_lifecycle(update_id: Uuid) -> bool {
    tracing::warn!(
        update_id = %update_id,
        "durable update admission reached an inspectable lifecycle boundary"
    );
    false
}

fn log_generation_race(update_id: Uuid) -> bool {
    // A retry can race the generation snapshot with the previous hook's
    // cleanup. The row remains draining and the next bounded reconciliation
    // observes the committed run count and derives the next identity.
    tracing::debug!(
        update_id = %update_id,
        "durable update admission generation raced; deferring"
    );
    false
}

enum AdmissionFailureKind {
    DrainPending,
    AuthorizationDenied,
    InvalidLifecycle,
    GenerationRace,
    Other,
}

const fn admission_failure_kind(error: &ReleaseServiceError) -> AdmissionFailureKind {
    if matches!(error, ReleaseServiceError::UpdateDrainPending) {
        return AdmissionFailureKind::DrainPending;
    }
    if matches!(error, ReleaseServiceError::AuthorizationDenied) {
        return AdmissionFailureKind::AuthorizationDenied;
    }
    if matches!(error, ReleaseServiceError::InvalidUpdateLifecycle) {
        return AdmissionFailureKind::InvalidLifecycle;
    }
    if matches!(error, ReleaseServiceError::UpdateAdmissionGenerationRace) {
        return AdmissionFailureKind::GenerationRace;
    }
    AdmissionFailureKind::Other
}

#[async_trait]
impl RunCompletionObserver for UpdateRunCompletion {
    async fn after_cleanup(&self, run: &Run) -> Result<(), RunCompletionError> {
        self.apply(run).await?;
        if run.kind == RunKind::Normal {
            self.resume_pending_admissions().await?;
        }
        Ok(())
    }

    async fn recover(&self) -> Result<usize, RunCompletionError> {
        let run_ids = recoverable_update_hook_run_ids(&self.pool)
            .await
            .map_err(completion_error)?;
        let mut recovered = 0;
        for run_id in run_ids {
            self.releases
                .reconcile_update_run(RunId::from_uuid(run_id))
                .await
                .map_err(completion_error)?;
            recovered += 1;
        }
        recovered += self.resume_pending_admissions().await?;
        Ok(recovered)
    }
}

/// Derives a new hook run identity for each durable attempt generation.
///
/// The generation is persisted indirectly by the update-run history, so a
/// recovered retry cannot collide with the run that preceded it.
pub fn deterministic_update_hook_run_id(update_id: Uuid, generation: i64) -> RunId {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus:update-hook-run-v2\0");
    digest.update(update_id.as_bytes());
    digest.update(generation.to_be_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.finalize()[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    RunId::from_uuid(Uuid::from_bytes(bytes))
}

/// Reconciles accepted and retry-scheduled updates after their triggering
/// normal-run cleanup callback has already fired.
pub async fn update_admission_reconciliation_loop(
    observer: Arc<UpdateRunCompletion>,
    cancellation: CancellationToken,
    configured_interval: Duration,
    ready: oneshot::Sender<()>,
) {
    if ready.send(()).is_err() {
        return;
    }
    let interval = configured_interval.max(Duration::from_secs(1));
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            () = cancellation.cancelled() => break,
            _ = ticker.tick() => {
                match tokio::time::timeout(
                    Duration::from_secs(1),
                    observer.resume_pending_admissions(),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(%error, "update admission reconciliation deferred");
                    }
                    Err(_) => {
                        tracing::warn!("update admission reconciliation timed out");
                    }
                }
            }
        }
    }
}
