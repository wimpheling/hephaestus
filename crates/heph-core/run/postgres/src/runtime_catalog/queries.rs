use run_orchestrator::{MailboxRuntimeEvent, RunRuntimeArtifact, RunRuntimeCatalogError};
use runtime_types::RunId;
use uuid::Uuid;

use crate::PgRunRepository;

use super::{
    models::{MailboxRuntimeRow, RuntimeArtifactRow},
    utility::storage,
};

impl PgRunRepository {
    pub(super) async fn load_mailbox_event(
        &self,
        run_id: RunId,
    ) -> Result<Option<MailboxRuntimeEvent>, RunRuntimeCatalogError> {
        let row = sqlx::query_as::<_, MailboxRuntimeRow>(
            "WITH RECURSIVE retry_lineage(run_id, depth, visited) AS (
                 SELECT $1::uuid, 0, ARRAY[$1::uuid]
                 UNION ALL
                 SELECT request.retry_of_run_id, lineage.depth + 1,
                        lineage.visited || request.retry_of_run_id
                   FROM retry_lineage AS lineage
                   JOIN run_requests AS request ON request.run_id = lineage.run_id
                 WHERE request.retry_of_run_id IS NOT NULL
                    AND lineage.depth < 64
                    AND NOT request.retry_of_run_id = ANY(lineage.visited)
             )
             SELECT event.mailbox_id, event.id AS event_id, event.body_id,
                    event.method, event.route, event.selected_headers,
                    event.content_type, event.trace_context, event.received_at,
                    payload.encoded_body, payload.integrity_hash
               FROM retry_lineage AS lineage
               JOIN mailbox_delivery_attempts AS attempt
                 ON attempt.run_id = lineage.run_id
               JOIN mailbox_events AS event ON event.id = attempt.event_id
               JOIN mailbox_payloads AS payload ON payload.id = event.body_id
              ORDER BY lineage.depth ASC
              LIMIT 1",
        )
        .bind(run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        row.map(TryInto::try_into).transpose()
    }

    pub(super) async fn is_retry_run(&self, run_id: RunId) -> Result<bool, RunRuntimeCatalogError> {
        sqlx::query_scalar(
            "SELECT retry_of_run_id IS NOT NULL
               FROM run_requests
              WHERE run_id = $1",
        )
        .bind(run_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)
        .map(|value| value.unwrap_or(false))
    }

    pub(super) async fn validate_retry_lineage(
        &self,
        run_id: RunId,
    ) -> Result<(), RunRuntimeCatalogError> {
        let (terminated, cyclic, exhausted): (bool, bool, bool) = sqlx::query_as(
            "WITH RECURSIVE retry_lineage(run_id, depth, visited, retry_of_run_id) AS (
                 SELECT request.run_id, 0, ARRAY[request.run_id], request.retry_of_run_id
                   FROM run_requests AS request
                  WHERE request.run_id = $1
                 UNION ALL
                 SELECT parent.run_id, lineage.depth + 1,
                        lineage.visited || parent.run_id, parent.retry_of_run_id
                   FROM retry_lineage AS lineage
                   JOIN run_requests AS parent
                     ON parent.run_id = lineage.retry_of_run_id
                  WHERE lineage.retry_of_run_id IS NOT NULL
                    AND lineage.depth < 64
                    AND NOT parent.run_id = ANY(lineage.visited)
             )
             SELECT EXISTS (SELECT 1 FROM retry_lineage WHERE retry_of_run_id IS NULL),
                    EXISTS (SELECT 1 FROM retry_lineage AS lineage
                              WHERE lineage.retry_of_run_id = ANY(lineage.visited)),
                    EXISTS (SELECT 1 FROM retry_lineage
                              WHERE depth >= 64 AND retry_of_run_id IS NOT NULL)",
        )
        .bind(run_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(storage)?;
        if !terminated || cyclic || exhausted {
            return Err(RunRuntimeCatalogError::InvalidData(
                "retry run lineage is invalid",
            ));
        }
        Ok(())
    }

    pub(super) async fn load_runtime_artifacts(
        &self,
        release_id: Uuid,
    ) -> Result<Vec<RunRuntimeArtifact>, RunRuntimeCatalogError> {
        sqlx::query_as::<_, RuntimeArtifactRow>(
            "SELECT path, kind, mode, content_hash, size_bytes, storage_key
             FROM release_artifacts
             WHERE release_id = $1
               AND kind IN ('executable', 'file', 'manifest')
             ORDER BY path, id",
        )
        .bind(release_id)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }
}
