//! Scoped terminal comparison; no live authorization or lifecycle mutation.

#[path = "expired/graph.rs"]
mod graph;
#[path = "expired/recheck.rs"]
mod recheck;
#[path = "expired/scope.rs"]
mod scope;
#[cfg(test)]
#[path = "expired/tests.rs"]
mod tests;

use crate::{PgRuntimeGitCredentialRepository, storage};
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId};
use runtime_authority::{ExpiredRuntimeHandoff, RuntimeAuthorityError, RuntimeSessionRepository};
use runtime_git_authority::{
    ExpiredRuntimeGitCeiling, ExpiredRuntimeGitHandoff, ExpiredRuntimeGitSessionMetadata,
    RuntimeGitAuthorityError,
};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

impl PgRuntimeGitCredentialRepository {
    pub(super) async fn observe_expired_original(
        &self,
        run: Uuid,
        session: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Result<ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError> {
        let observer = self
            .expired_original
            .as_ref()
            .ok_or(RuntimeGitAuthorityError::Unsupported)?;
        // Finish the generic observer before holding a Git connection: this also
        // works with a single-connection original pool. These are separate snapshots.
        let original = observer
            .observe_expired_run_handoff(run, session, generation)
            .await
            .map_err(generic_error)?;
        if !original.matches_request(run, session, generation) {
            return Err(RuntimeGitAuthorityError::IdentityMismatch);
        }
        let mut tx = self.pool.begin().await.map_err(storage)?;
        let result = observe(&mut tx, &original).await;
        let rollback = tx.rollback().await.map_err(storage);
        match result {
            Err(cause) => Err(cause),
            Ok(handoff) => {
                rollback?;
                Ok(handoff)
            }
        }
    }
}

async fn observe(
    tx: &mut Transaction<'_, Postgres>,
    original: &ExpiredRuntimeHandoff,
) -> Result<ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError> {
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    sqlx::query("SET LOCAL search_path = pg_catalog, public, pg_temp")
        .execute(&mut **tx)
        .await
        .map_err(storage)?;
    let context: (String, String, String) = sqlx::query_as(
        "SELECT current_user::text,current_setting('transaction_read_only'),
                current_setting('transaction_isolation')",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(storage)?;
    if context.0 != "hephaestus_worker" || context.1 != "on" || context.2 != "repeatable read" {
        return Err(RuntimeGitAuthorityError::Unsupported);
    }
    recheck::original(tx, original).await?;
    let observed_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **tx)
        .await
        .map_err(storage)?;
    let ceiling = ceiling(original, observed_at)?;
    graph::observe(tx, original, ceiling).await
}

fn ceiling(
    original: &ExpiredRuntimeHandoff,
    observed_at: OffsetDateTime,
) -> Result<ExpiredRuntimeGitCeiling, RuntimeGitAuthorityError> {
    let session = original.session();
    ExpiredRuntimeGitCeiling::new(
        original.pins().run(),
        ExpiredRuntimeGitSessionMetadata {
            session: session.id,
            snapshot: session.snapshot_id,
            identity_hash: session.identity_hash,
            generation: session.generation,
            status: session.status,
            issued_at: session.issued_at,
            expires_at: session.expires_at,
            acknowledged_at: session.acknowledged_at,
            revoked_at: session.revoked_at,
        },
        original.snapshot().clone(),
        original.identity().clone(),
        original.credential_hash(),
        observed_at,
    )
}

const fn generic_error(error: RuntimeAuthorityError) -> RuntimeGitAuthorityError {
    match error {
        RuntimeAuthorityError::Unsupported => RuntimeGitAuthorityError::Unsupported,
        RuntimeAuthorityError::NotFound => RuntimeGitAuthorityError::NotFound,
        RuntimeAuthorityError::GenerationMismatch | RuntimeAuthorityError::IdentityMismatch => {
            RuntimeGitAuthorityError::IdentityMismatch
        }
        _ => RuntimeGitAuthorityError::Persistence,
    }
}
