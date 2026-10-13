//! Worker-only saved-empty verification without secret dispatch or mount IO.

use super::{PostgresSecretMountMetadata, db_error, redacted};
use run_domain::{Run, RunKind, RunState};
use run_orchestrator::RunSecretError;
use sqlx::Acquire;

impl PostgresSecretMountMetadata {
    pub(super) async fn verify_invocation_empty(&self, run: &Run) -> Result<(), RunSecretError> {
        let configuration = self
            .invocation_configuration
            .as_ref()
            .ok_or_else(|| redacted("qualified empty Invocation secrets are unsupported"))?;
        if run.kind != RunKind::Invocation
            || run.attachment_id.is_some()
            || run.volume_id.is_some()
            || run.lease_id.is_some()
            || run.lease_fencing_token.is_some()
        {
            return Err(redacted("qualified empty Invocation pins differ"));
        }
        let phase = match run.state {
            RunState::Provisioning => "provisioning",
            RunState::Starting => "starting",
            RunState::Running => "running",
            _ => return Err(redacted("qualified empty Invocation phase is unavailable")),
        };
        let config = serde_json::to_vec(&configuration.canonical_json())
            .map_err(|_| redacted("qualified empty Invocation configuration is unavailable"))?;
        let mut tx = self.pool.begin().await.map_err(db_error)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        let verified: bool = sqlx::query_scalar(
            "SELECT verify_invocation119_empty_secrets($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(run.id.as_uuid())
        .bind(config)
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid())
        .bind(run.release_agent_id.as_uuid())
        .bind(run.command_id.as_uuid())
        .bind(run.requires_state)
        .bind(run.vm_id.as_deref())
        .bind(phase)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_error)?;
        if !verified {
            return Err(redacted(
                "qualified saved-empty Invocation evidence is unavailable",
            ));
        }
        tx.commit().await.map_err(db_error)
    }

    pub(super) async fn invocation_cleanup_empty(
        &self,
        run_id: runtime_types::RunId,
    ) -> Result<bool, RunSecretError> {
        let mut connection = self.pool.acquire().await.map_err(db_error)?;
        let mut tx = connection.begin().await.map_err(db_error)?;
        let decision: Result<bool, RunSecretError> = async {
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            let kind: Option<String> = sqlx::query_scalar("SELECT run_kind FROM runs WHERE id=$1")
                .bind(run_id.as_uuid())
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_error)?;
            match kind.as_deref() {
                Some("normal" | "update") => return Ok(false),
                Some("invocation") => {}
                _ => return Err(redacted("secret cleanup run identity is unavailable")),
            }
            let configuration = self
                .invocation_configuration
                .as_ref()
                .ok_or_else(|| redacted("qualified empty Invocation secrets are unsupported"))?;
            let config = serde_json::to_vec(&configuration.canonical_json()).map_err(|_| {
                redacted("qualified empty Invocation cleanup configuration is unavailable")
            })?;
            let provider = configuration.provider_scope();
            let verified: serde_json::Value =
                sqlx::query_scalar("SELECT verify_invocation142_empty_secret_cleanup($1,$2,$3,$4)")
                    .bind(run_id.as_uuid())
                    .bind(config)
                    .bind(provider.namespace())
                    .bind(provider.host_id())
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db_error)?;
            let expected = serde_json::json!({"decision": "empty", "run_id": run_id.as_uuid()});
            if verified != expected {
                return Err(redacted(
                    "qualified saved-empty Invocation cleanup evidence is unavailable",
                ));
            }
            Ok(true)
        }
        .await;
        let empty = match decision {
            Ok(empty) => empty,
            Err(error) => {
                let _ = tx.rollback().await;
                drop(connection);
                return Err(error);
            }
        };
        if let Err(error) = tx.commit().await {
            // SQLx consumes the transaction on commit failure; retain this same
            // connection to await cleanup, keeping the original redacted error.
            let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
            drop(connection);
            return Err(db_error(error));
        }
        drop(connection);
        Ok(empty)
    }
}
