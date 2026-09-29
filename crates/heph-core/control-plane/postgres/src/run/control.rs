//! Durable run control request creation.

use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use super::model::{
    ControlKind, ControlTarget, RequestControl, RequestedControl, RunApplication, RunError,
};

impl RunApplication {
    pub async fn request_control(
        &self,
        identity: &AuthenticatedIdentity,
        request: RequestControl,
    ) -> Result<RequestedControl, RunError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(RunError::Persistence)?;
        let kind = match request.kind {
            ControlKind::Cancel => "cancel_run",
            ControlKind::Retry => "retry_run",
            ControlKind::Approve => "approve_result",
            ControlKind::Reject => "reject_result",
        };
        let (run_id, proposal_id) = match request.target {
            ControlTarget::Run(id) => (Some(id), None),
            ControlTarget::Proposal(id) => (None, Some(id)),
        };
        if let Some(row) = sqlx::query_as::<_, ControlRow>("SELECT id, kind, repository_id, run_id, proposal_id, reason, state FROM control_requests WHERE actor_id = $1 AND request_id = $2")
            .bind(identity.user_id.as_uuid()).bind(identity.idempotency_id.as_uuid()).fetch_optional(&mut *tx).await.map_err(RunError::Persistence)? {
            if row.kind != kind || row.repository_id != request.repository_id || row.run_id != run_id || row.proposal_id != proposal_id || row.reason != request.reason { return Err(RunError::IdempotencyConflict); }
            tx.commit().await.map_err(RunError::Persistence)?;
            return Ok(RequestedControl { id: row.id, state: row.state });
        }
        let id = stable_id(identity, kind);
        let state = sqlx::query_scalar::<_, String>(
            "INSERT INTO control_requests
             (id, kind, actor_id, request_id, repository_id, run_id, proposal_id,
              reason, state, diagnostics, processed_at)
             SELECT $1, $2, $3, $4, $5, $6, $7, $8,
                    CASE WHEN $2 = 'retry_run'
                               AND NOT EXISTS (
                                   SELECT 1 FROM run_requests
                                   WHERE run_id = $6
                               )
                         THEN 'failed' ELSE 'pending' END,
                    CASE WHEN $2 = 'retry_run'
                               AND NOT EXISTS (
                                   SELECT 1 FROM run_requests
                                   WHERE run_id = $6
                               )
                         THEN jsonb_build_array(
                                  jsonb_build_object('code', 'retry_unsupported'))
                         ELSE '[]'::jsonb END,
                    CASE WHEN $2 = 'retry_run'
                               AND NOT EXISTS (
                                   SELECT 1 FROM run_requests
                                   WHERE run_id = $6
                               )
                         THEN now() ELSE NULL END
             RETURNING state",
        )
        .bind(id)
        .bind(kind)
        .bind(identity.user_id.as_uuid())
        .bind(identity.idempotency_id.as_uuid())
        .bind(request.repository_id)
        .bind(run_id)
        .bind(proposal_id)
        .bind(request.reason)
        .fetch_one(&mut *tx)
        .await
        .map_err(RunError::Persistence)?;
        tx.commit().await.map_err(RunError::Persistence)?;
        Ok(RequestedControl { id, state })
    }
}

#[derive(FromRow)]
struct ControlRow {
    id: Uuid,
    kind: String,
    repository_id: Uuid,
    run_id: Option<Uuid>,
    proposal_id: Option<Uuid>,
    reason: String,
    state: String,
}

fn stable_id(identity: &AuthenticatedIdentity, kind: &str) -> Uuid {
    let mut hash = Sha256::new();
    hash.update(b"hephaestus.control-request.v1\0");
    hash.update(identity.user_id.as_uuid().as_bytes());
    hash.update(identity.idempotency_id.as_uuid().as_bytes());
    hash.update(kind.as_bytes());
    let mut bytes: [u8; 16] = hash.finalize()[..16]
        .try_into()
        .expect("fixed SHA-256 prefix");
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}
