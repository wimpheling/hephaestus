//! Trusted worker outcome boundary; caller DTOs are comparisons, not proof.
use crate::{execution_rows::ClaimWire, repository_error};
use recipe_application::{DeploymentAttemptId, DeploymentError, DiagnosticCode, EffectClaim};
use serde::Serialize;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// Closed authoritative observation made by an owning provider adapter.
///
/// A worker records these only after it verifies exact identity, configuration,
/// and action-specific safety. Creation additionally requires durable provenance
/// tying ownership to the original operation, recoverable after a crash. Matching
/// predicted identity and configuration cannot adopt a preexisting independent
/// resource. Deletion must verify that same ownership proof. A missing row or
/// expired claim is not absence proof; absence must exclude a matching unowned
/// resource as well. Provider adapters must establish these facts before recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum VerifiedOutcome {
    /// Exact identity and required safety prove the action succeeded.
    Applied,
    /// Action failed; this does not establish absence or permit a creation retry.
    Failed(DiagnosticCode),
    /// Authoritative observation proves the original action was not applied.
    NotApplied,
    /// Action outcome or detach/fencing safety remains unproven.
    Unproven(DiagnosticCode),
}

impl VerifiedOutcome {
    pub(crate) const fn fields(self) -> (&'static str, Option<DiagnosticCode>) {
        match self {
            Self::Applied => ("applied", None),
            Self::Failed(code) => ("failed", Some(code)),
            Self::NotApplied => ("not_applied", None),
            Self::Unproven(code) => ("unproven", Some(code)),
        }
    }
}

/// Records verified observations using a trusted worker database role.
///
/// This is a backend composition boundary, never a public CLI/RPC endpoint.
/// Actor pools cannot insert verification receipts. This adapter performs no
/// provider observation itself; the owning provider must establish its proof.
#[derive(Clone)]
pub struct PostgresEffectVerificationRecorder {
    pool: PgPool,
}

impl PostgresEffectVerificationRecorder {
    /// Creates a recorder over a pool configured as `hephaestus_worker`.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Appends an exact verified outcome, independently of the actor's progress update.
    ///
    /// `verification_attempt` is the claim attempt for completion, or the fresh
    /// reconciliation attempt. A new receipt must match the current active fence.
    /// Unchanged retry returns the original receipt ID after that fence closes;
    /// this historical receipt does not authorize another progress transition.
    ///
    /// # Errors
    /// Rejects actor roles, forged claims, new observations outside the active fence,
    /// changed proof input, or persistence failure.
    pub async fn record(
        &self,
        claim: &EffectClaim,
        verification_attempt: DeploymentAttemptId,
        outcome: VerifiedOutcome,
    ) -> Result<Uuid, DeploymentError> {
        let mut tx = self.pool.begin().await.map_err(repository_error)?;
        let role: String = sqlx::query_scalar("SELECT current_user")
            .fetch_one(&mut *tx)
            .await
            .map_err(repository_error)?;
        if role != "hephaestus_worker" {
            return Err(DeploymentError::AuthorizationDenied);
        }
        let wire = ClaimWire::new(claim)?;
        let stored: sqlx::types::Json<ClaimWire> =
            sqlx::query_scalar("SELECT claim_json FROM recipe_effect_attempts WHERE id = $1")
                .bind(claim.provenance.attempt_id.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(repository_error)?
                .ok_or(DeploymentError::Unavailable)?;
        if stored.0 != wire {
            return Err(DeploymentError::InputConflict);
        }
        let (kind, diagnostic) = outcome.fields();
        let existing: Option<(Uuid, String, Option<String>)> = sqlx::query_as(
            "SELECT id, outcome, diagnostic FROM recipe_effect_verifications WHERE attempt_id = $1 AND verification_attempt_id = $2",
        ).bind(wire.attempt_id).bind(verification_attempt.as_uuid()).fetch_optional(&mut *tx).await.map_err(repository_error)?;
        if let Some((id, previous, code)) = existing {
            if previous != kind || code.as_deref() != diagnostic.map(DiagnosticCode::as_str) {
                return Err(DeploymentError::InputConflict);
            }
            tx.commit().await.map_err(repository_error)?;
            return Ok(id);
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO recipe_effect_verifications (id, attempt_id, verification_attempt_id, deployment_id,
            project_id, resource_name, generation, action, identity_json, input_hash, outcome, diagnostic)
            SELECT $1, id, $2, deployment_id, project_id, resource_name, generation, action,
                claim_json->'identity', input_hash, $3, $4 FROM recipe_effect_attempts WHERE id = $5")
            .bind(id).bind(verification_attempt.as_uuid()).bind(kind).bind(diagnostic.map(DiagnosticCode::as_str))
            .bind(wire.attempt_id).execute(&mut *tx).await.map_err(repository_error)?;
        tx.commit().await.map_err(repository_error)?;
        Ok(id)
    }
}

pub async fn matching(
    tx: &mut Transaction<'_, Postgres>,
    claim: &EffectClaim,
    verification: DeploymentAttemptId,
    kind: &str,
    diagnostic: Option<DiagnosticCode>,
) -> Result<Uuid, DeploymentError> {
    let row: (Uuid, String, Option<String>) = sqlx::query_as(
        "SELECT id, outcome, diagnostic FROM recipe_effect_verifications
        WHERE attempt_id = $1 AND verification_attempt_id = $2 AND generation = $3 AND action = $4
          AND identity_json = $5 AND input_hash = $6",
    )
    .bind(claim.provenance.attempt_id.as_uuid())
    .bind(verification.as_uuid())
    .bind(i64::try_from(claim.generation).map_err(|_| DeploymentError::StaleClaim)?)
    .bind(claim.action.as_str())
    .bind(crate::execution_rows::identity(claim.identity)?)
    .bind(claim.input_hash.as_bytes().as_slice())
    .fetch_optional(&mut **tx)
    .await
    .map_err(repository_error)?
    .ok_or(DeploymentError::ReconciliationRequired)?;
    if row.1 != kind || row.2.as_deref() != diagnostic.map(DiagnosticCode::as_str) {
        return Err(DeploymentError::InputConflict);
    }
    Ok(row.0)
}
