//! Review control and approval transaction orchestration.

use async_trait::async_trait;
use authz_domain::{ObjectRef, ObjectType, Permission};
use review_domain::{ControlCommand, ControlKind};
use review_service::{
    ApprovalDisposition, ApprovalPreparation, ApprovalProposal, ControlOutcome, ReviewRepository,
    ReviewRepositoryError,
};

use super::PostgresReviewRepository;
use super::commands::{cancel_run, reject_result, retry_run};
use super::models::{ControlRow, ProposalRow};
use super::support::db_error;
use super::transaction::{append_run_event, authorize, close_denied, complete_control, set_actor};
use runtime_types::RunId;

#[async_trait]
impl ReviewRepository for PostgresReviewRepository {
    async fn execute_control(
        &self,
        command: &ControlCommand,
    ) -> Result<ControlOutcome, ReviewRepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(db_error)?;
        set_actor(&mut transaction, command)
            .await
            .map_err(db_error)?;
        let row = ControlRow::load(&mut transaction, command.command_id).await?;
        row.matches(command)?;
        if row.is_terminal() {
            transaction.commit().await.map_err(db_error)?;
            return Ok(ControlOutcome::AlreadyCompleted);
        }
        let outcome = match command.kind {
            ControlKind::CancelRun => {
                cancel_run(&self.authorizer, &mut transaction, command).await?
            }
            ControlKind::RetryRun => retry_run(&self.authorizer, &mut transaction, command).await?,
            ControlKind::RejectResult => {
                reject_result(&self.authorizer, &mut transaction, command).await?
            }
            ControlKind::ApproveResult => {
                return Err(ReviewRepositoryError::Infrastructure(
                    "approval must use prepare_approval".to_owned(),
                ));
            }
        };
        transaction.commit().await.map_err(db_error)?;
        Ok(outcome)
    }

    async fn prepare_approval(
        &self,
        command: &ControlCommand,
    ) -> Result<ApprovalPreparation, ReviewRepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(db_error)?;
        set_actor(&mut transaction, command)
            .await
            .map_err(db_error)?;
        let control = ControlRow::load(&mut transaction, command.command_id).await?;
        control.matches(command)?;
        if control.is_terminal() {
            transaction.commit().await.map_err(db_error)?;
            return Ok(ApprovalPreparation::Terminal(
                ControlOutcome::AlreadyCompleted,
            ));
        }
        let proposal_id = command
            .proposal_id
            .ok_or(ReviewRepositoryError::DeliveryMismatch)?;
        let proposal = ProposalRow::load(&mut transaction, proposal_id).await?;
        proposal.matches(command)?;
        let decision = authorize(
            &self.authorizer,
            &mut transaction,
            command,
            Permission::CanWrite,
            ObjectRef::new(ObjectType::Repository, proposal.repository_id),
        )
        .await?;
        if !decision.is_allowed() {
            close_denied(&mut transaction, command.command_id).await?;
            transaction.commit().await.map_err(db_error)?;
            return Ok(ApprovalPreparation::Terminal(ControlOutcome::Denied));
        }
        match proposal.state.as_str() {
            "open" | "approval_requested" => {}
            "approved" => {
                complete_control(&mut transaction, command.command_id).await?;
                transaction.commit().await.map_err(db_error)?;
                return Ok(ApprovalPreparation::Terminal(
                    ControlOutcome::AlreadyCompleted,
                ));
            }
            state => return Err(ReviewRepositoryError::ProposalClosed(state.to_owned())),
        }
        sqlx::query("UPDATE control_requests SET state = 'processing' WHERE id = $1")
            .bind(command.command_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(db_error)?;
        sqlx::query(
            "UPDATE review_proposals
             SET state = 'approval_requested', version = version + 1,
                 decision_actor_id = $2, decision_request_id = $3,
                 decision_reason = $4, updated_at = now()
             WHERE id = $1 AND state IN ('open', 'approval_requested')",
        )
        .bind(proposal.id)
        .bind(command.actor_id.as_uuid())
        .bind(command.request_id.as_uuid())
        .bind(&command.reason)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
        transaction.commit().await.map_err(db_error)?;
        Ok(ApprovalPreparation::Ready(proposal.into_service()))
    }

    async fn finalize_approval(
        &self,
        command: &ControlCommand,
        proposal: &ApprovalProposal,
        disposition: ApprovalDisposition,
    ) -> Result<ControlOutcome, ReviewRepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(db_error)?;
        set_actor(&mut transaction, command)
            .await
            .map_err(db_error)?;
        let control = ControlRow::load(&mut transaction, command.command_id).await?;
        control.matches(command)?;
        if control.is_terminal() {
            transaction.commit().await.map_err(db_error)?;
            return Ok(ControlOutcome::AlreadyCompleted);
        }
        let locked = ProposalRow::load(&mut transaction, proposal.id).await?;
        locked.matches(command)?;
        if locked.state == "approved" || locked.state == "conflicted" {
            transaction.commit().await.map_err(db_error)?;
            return Ok(ControlOutcome::AlreadyCompleted);
        }
        if locked.state != "approval_requested"
            || locked.input_commit != proposal.input_commit
            || locked.result_commit != proposal.result_commit
        {
            return Err(ReviewRepositoryError::DeliveryMismatch);
        }
        let (state, event_type, outcome) = match disposition {
            ApprovalDisposition::Approved => {
                ("approved", "review.approved", ControlOutcome::Completed)
            }
            ApprovalDisposition::Conflicted => (
                "conflicted",
                "review.conflicted",
                ControlOutcome::Conflicted,
            ),
        };
        sqlx::query(
            "UPDATE review_proposals
             SET state = $2, version = version + 1, decided_at = now(), updated_at = now()
             WHERE id = $1 AND state = 'approval_requested'",
        )
        .bind(locked.id)
        .bind(state)
        .execute(&mut *transaction)
        .await
        .map_err(db_error)?;
        complete_control(&mut transaction, command.command_id).await?;
        append_run_event(
            &mut transaction,
            RunId::from_uuid(locked.run_id),
            event_type,
            serde_json::json!({
                "actor_id": command.actor_id,
                "target_ref": locked.target_ref,
                "input_commit": locked.input_commit,
                "result_commit": locked.result_commit,
            }),
        )
        .await?;
        transaction.commit().await.map_err(db_error)?;
        Ok(outcome)
    }
}
