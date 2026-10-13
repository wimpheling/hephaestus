//! Worker-only immutable expired observation; no lifecycle or filesystem effects.

#[path = "expired/bindings.rs"]
mod bindings;
#[path = "expired/original.rs"]
mod original;
#[path = "expired/session.rs"]
mod session;
#[cfg(test)]
#[path = "expired/tests.rs"]
mod tests;

use super::{agent_repository::PgRuntimeSessionRepository, common::storage};
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId};
use runtime_authority::{ExpiredRuntimeHandoff, RuntimeAuthorityError};
use sqlx::{Postgres, Transaction};
use time::OffsetDateTime;
use uuid::Uuid;

impl PgRuntimeSessionRepository {
    pub(super) async fn observe_expired_original(
        &self,
        run: Uuid,
        session: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Result<ExpiredRuntimeHandoff, RuntimeAuthorityError> {
        if self.expired_legacy_scope.is_none() && self.invocation_configuration.is_none() {
            return Err(RuntimeAuthorityError::Unsupported);
        }
        if run.is_nil() || session.as_uuid().is_nil() {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        let mut tx = self.pool.begin().await.map_err(storage)?;
        // Every ordinary result, including setup/role/decode errors, reaches an
        // awaited rollback. Cancellation is not an awaited rollback guarantee.
        let result = self
            .expired_in_transaction(&mut tx, run, session, generation)
            .await;
        let rollback = tx.rollback().await.map_err(storage);
        match result {
            Err(cause) => Err(cause),
            Ok(original) => {
                rollback?;
                Ok(original)
            }
        }
    }

    async fn expired_in_transaction(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        run: Uuid,
        session: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Result<ExpiredRuntimeHandoff, RuntimeAuthorityError> {
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
        ensure_context(&context)?;
        let original = original::load(tx, run).await?;
        original::qualify(self, tx, &original, session).await?;
        let session = session::load(tx, &original, session, generation).await?;
        let snapshot = bindings::load(tx, &original, &session).await?;
        let observed_at: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(storage)?;
        session.project(&original, snapshot, observed_at)
    }
}

fn ensure_context(context: &(String, String, String)) -> Result<(), RuntimeAuthorityError> {
    if context.0 == "hephaestus_worker" && context.1 == "on" && context.2 == "repeatable read" {
        Ok(())
    } else {
        Err(RuntimeAuthorityError::Unsupported)
    }
}
