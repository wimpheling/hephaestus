use crate::prepared_instance_safety;
use identity_domain::AuthenticatedIdentity;
use recipe_application::DeploymentError;
use sqlx::{PgPool, Postgres, Transaction};

pub async fn worker(pool: &PgPool) -> Result<Transaction<'_, Postgres>, DeploymentError> {
    pool.begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(|error| storage(&error))
}
pub async fn actor(pool: &PgPool) -> Result<Transaction<'_, Postgres>, DeploymentError> {
    pool.begin_with("BEGIN ISOLATION LEVEL READ COMMITTED READ WRITE")
        .await
        .map_err(|error| storage(&error))
}
pub async fn setup(
    tx: &mut Transaction<'_, Postgres>,
    expected_role: &str,
    identity: Option<&AuthenticatedIdentity>,
) -> Result<(), DeploymentError> {
    prepared_instance_safety::role(tx, expected_role).await?;
    sqlx::query("SELECT set_config('search_path','pg_catalog,public,pg_temp',true)")
        .execute(&mut **tx)
        .await
        .map_err(|error| storage(&error))?;
    if let Some(identity) = identity {
        sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),set_config('hephaestus.request_id',$2,true),set_config('hephaestus.occurrence_id',$3,true)")
            .bind(identity.user_id.to_string()).bind(identity.request_id.to_string())
            .bind(identity.idempotency_id.to_string()).execute(&mut **tx)
            .await.map_err(|error| storage(&error))?;
    }
    Ok(())
}
pub async fn finish<T>(
    tx: Transaction<'_, Postgres>,
    result: Result<T, DeploymentError>,
    commit: bool,
) -> Result<T, DeploymentError> {
    match result {
        Ok(value) => {
            if commit {
                tx.commit().await
            } else {
                tx.rollback().await
            }
            .map_err(|error| storage(&error))?;
            Ok(value)
        }
        Err(primary) => {
            // A failed observation/write never substitutes rollback failure for
            // the actual provenance/authority/native validation cause.
            let _ = tx.rollback().await;
            Err(primary)
        }
    }
}
pub fn storage(error: &sqlx::Error) -> DeploymentError {
    match error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .as_deref()
    {
        Some("42501") => DeploymentError::AuthorizationDenied,
        Some("P1180" | "P1420" | "P1430") => DeploymentError::ReconciliationRequired,
        Some("40001" | "55P03") => DeploymentError::StaleClaim,
        _ => DeploymentError::Unavailable,
    }
}
