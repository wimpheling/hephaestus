//! Read-only owned-profile requests; no credentials or handoff are created here.

use capability_domain::AuthorizationSnapshot;
use heph_run::{Run, RunAuthorityError, RunState};
use runtime_authority::{
    LiveRunSessionCeiling, LiveRunSnapshotPins, LiveRunSnapshotRequest, LiveRunSnapshotResolver,
    RuntimeAuthorityError,
};
use runtime_authority_postgres::PgRuntimeSessionRepository;
use time::OffsetDateTime;

use super::authority_error;

pub async fn snapshot(
    repository: &PgRuntimeSessionRepository,
    run: &Run,
) -> Result<AuthorizationSnapshot, RunAuthorityError> {
    let request = request_for_run(repository, run).await?;
    repository
        .resolve_live_snapshot(
            &request,
            authz_postgres::AUTHORIZATION_MODEL_VERSION,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(authority_error)
}

pub async fn request_for_run(
    repository: &PgRuntimeSessionRepository,
    run: &Run,
) -> Result<LiveRunSnapshotRequest, RunAuthorityError> {
    let pins = LiveRunSnapshotPins::new(
        run.id.as_uuid(),
        run.instance_id.as_uuid(),
        run.instance_revision_id.as_uuid(),
        run.release_id.as_uuid(),
        run.release_agent_id.as_uuid(),
        run.command_id.as_uuid(),
    )
    .map_err(authority_error)?;
    let recorded = repository
        .recorded_live_session_ceiling(pins)
        .await
        .map_err(authority_error)?;
    request(pins, run.state, recorded).map_err(authority_error)
}

fn request(
    pins: LiveRunSnapshotPins,
    state: RunState,
    recorded: Option<LiveRunSessionCeiling>,
) -> Result<LiveRunSnapshotRequest, RuntimeAuthorityError> {
    match recorded {
        Some(ceiling) => LiveRunSnapshotRequest::issued(pins, ceiling),
        None if matches!(state, RunState::LeasingVolume | RunState::Provisioning) => {
            Ok(LiveRunSnapshotRequest::pre_issuance(pins))
        }
        None => Err(RuntimeAuthorityError::Unsupported),
    }
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;
