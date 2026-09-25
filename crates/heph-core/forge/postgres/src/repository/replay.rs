use super::{
    helpers::storage,
    rows::{ExistingReceiveProvenance, RunRequestRow},
};
use forge_domain::{ReceiveId, RepositoryId};
use forge_service::{ForgeRepositoryError, ReceiveResult, RunRequest};
use release_domain::BuildRequestId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub async fn replay_receive(
    mut transaction: Transaction<'_, Postgres>,
    receive_id: ReceiveId,
    repository_id: RepositoryId,
    runtime_session: Option<Uuid>,
    runtime_attachment: Option<Uuid>,
    existing: ExistingReceiveProvenance,
) -> Result<ReceiveResult, ForgeRepositoryError> {
    if existing.repository != repository_id.as_uuid()
        || existing.runtime_session != runtime_session
        || existing.runtime_attachment != runtime_attachment
    {
        return Err(ForgeRepositoryError::ReceiveConflict(receive_id));
    }
    let rows = sqlx::query_as::<_, RunRequestRow>(
        "SELECT id, repository_id, commit_sha, git_ref, receive_id,
                        instance_id, instance_revision_id, release_id,
                        release_agent_id, attachment_id, run_id, command_id,
                        requires_state
                 FROM run_requests
                 WHERE receive_id = $1
                 ORDER BY created_at, id",
    )
    .bind(receive_id.as_uuid())
    .fetch_all(&mut *transaction)
    .await
    .map_err(storage)?;
    let run_requests = rows
        .into_iter()
        .map(RunRequest::try_from)
        .collect::<Result<Vec<_>, _>>()?;
    let invalid_configurations = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM agent_config_revisions
                 WHERE receive_id = $1 AND status = 'invalid'",
    )
    .bind(receive_id.as_uuid())
    .fetch_one(&mut *transaction)
    .await
    .map_err(storage)?;
    let build_requests = sqlx::query_scalar::<_, Uuid>(
        "SELECT source.build_request_id
                 FROM build_request_sources AS source
                 WHERE source.receive_id = $1
                 ORDER BY source.created_at, source.build_request_id",
    )
    .bind(receive_id.as_uuid())
    .fetch_all(&mut *transaction)
    .await
    .map_err(storage)?
    .into_iter()
    .map(BuildRequestId::from_uuid)
    .collect();
    transaction.commit().await.map_err(storage)?;
    Ok(ReceiveResult {
        receive_id,
        run_requests,
        build_requests,
        invalid_configurations: usize::try_from(invalid_configurations)
            .map_err(|_| ForgeRepositoryError::InvalidStoredData("invalid revision count"))?,
    })
}
