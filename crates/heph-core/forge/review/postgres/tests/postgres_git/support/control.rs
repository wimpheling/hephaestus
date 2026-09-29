use identity_domain::RequestId;
use review_domain::{ControlCommand, ControlKind, ControlRequestId, ReviewProposalId};
use runtime_types::RunId;
use sqlx::PgPool;

use super::fixture::Fixture;
use super::git::kind_name;

pub async fn insert_control(
    pool: &PgPool,
    fixture: &Fixture,
    kind: ControlKind,
    run_id: Option<RunId>,
    proposal_id: Option<ReviewProposalId>,
) -> ControlCommand {
    let command = ControlCommand {
        command_id: ControlRequestId::new(),
        kind,
        actor_id: fixture.actor_id,
        request_id: RequestId::new(),
        repository_id: fixture.repository_id,
        run_id,
        proposal_id,
        reason: String::from("fixture"),
    };
    sqlx::query(
        "INSERT INTO control_requests
         (id, kind, actor_id, request_id, repository_id, run_id, proposal_id, reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(command.command_id.as_uuid())
    .bind(kind_name(kind))
    .bind(command.actor_id.as_uuid())
    .bind(command.request_id.as_uuid())
    .bind(command.repository_id.as_uuid())
    .bind(command.run_id.map(RunId::as_uuid))
    .bind(command.proposal_id.map(ReviewProposalId::as_uuid))
    .bind(&command.reason)
    .execute(pool)
    .await
    .expect("control request");
    command
}
