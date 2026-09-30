//! SQL row models for review controls and proposals.

use forge_domain::RepositoryId;
use review_domain::{ControlCommand, ControlRequestId, ReviewProposalId};
use review_service::{ApprovalProposal, ReviewRepositoryError};
use runtime_types::RunId;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use super::support::{db_error, kind_name};

#[derive(Debug, FromRow)]
pub struct ControlRow {
    pub id: Uuid,
    pub kind: String,
    pub actor_id: Uuid,
    pub request_id: Uuid,
    pub repository_id: Uuid,
    pub run_id: Option<Uuid>,
    pub proposal_id: Option<Uuid>,
    pub reason: String,
    pub state: String,
}

impl ControlRow {
    pub async fn load(
        transaction: &mut Transaction<'_, Postgres>,
        id: ControlRequestId,
    ) -> Result<Self, ReviewRepositoryError> {
        sqlx::query_as(
            "SELECT id, kind, actor_id, request_id, repository_id,
                    run_id, proposal_id, reason, state
             FROM control_requests WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(db_error)?
        .ok_or(ReviewRepositoryError::MissingControl(id))
    }

    pub fn matches(&self, command: &ControlCommand) -> Result<(), ReviewRepositoryError> {
        if self.id != command.command_id.as_uuid()
            || self.kind != kind_name(command.kind)
            || self.actor_id != command.actor_id.as_uuid()
            || self.request_id != command.request_id.as_uuid()
            || self.repository_id != command.repository_id.as_uuid()
            || self.run_id != command.run_id.map(RunId::as_uuid)
            || self.proposal_id != command.proposal_id.map(ReviewProposalId::as_uuid)
            || self.reason != command.reason
        {
            return Err(ReviewRepositoryError::DeliveryMismatch);
        }
        Ok(())
    }

    pub fn is_terminal(&self) -> bool {
        self.state == "completed" || self.state == "failed"
    }
}

#[derive(Debug, FromRow)]
pub struct ProposalRow {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub run_id: Uuid,
    pub target_ref: String,
    pub input_commit: String,
    pub result_commit: String,
    pub result_ref: String,
    pub state: String,
}

impl ProposalRow {
    pub async fn load(
        transaction: &mut Transaction<'_, Postgres>,
        id: ReviewProposalId,
    ) -> Result<Self, ReviewRepositoryError> {
        sqlx::query_as(
            "SELECT id, repository_id, run_id, target_ref, input_commit,
                    result_commit, result_ref, state
             FROM review_proposals WHERE id = $1 FOR UPDATE",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(db_error)?
        .ok_or(ReviewRepositoryError::MissingProposal(id))
    }

    pub fn matches(&self, command: &ControlCommand) -> Result<(), ReviewRepositoryError> {
        if self.repository_id != command.repository_id.as_uuid() {
            return Err(ReviewRepositoryError::DeliveryMismatch);
        }
        Ok(())
    }

    pub fn into_service(self) -> ApprovalProposal {
        ApprovalProposal {
            id: ReviewProposalId::from_uuid(self.id),
            repository_id: RepositoryId::from_uuid(self.repository_id),
            run_id: RunId::from_uuid(self.run_id),
            target_ref: self.target_ref,
            input_commit: self.input_commit,
            result_commit: self.result_commit,
            result_ref: self.result_ref,
        }
    }
}

#[derive(Debug, FromRow)]
pub struct RetrySource {
    pub repository_id: Uuid,
    pub commit_sha: String,
    pub git_ref: String,
    pub receive_id: Uuid,
    pub instance_id: Uuid,
    pub instance_revision_id: Uuid,
    pub release_id: Uuid,
    pub release_agent_id: Uuid,
    pub attachment_id: Uuid,
    pub platform_policy_version: String,
    pub requires_state: bool,
}
