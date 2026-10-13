//! One-time observed database contract for explicit qualified Invocation startup.

mod journal;
mod routines;
mod secret_mounts;
mod sessions;

use sqlx::{PgPool, Postgres, Transaction};

/// Checks three held connections before qualified Invocation filesystem effects.
///
/// Requires the supported `PostgreSQL` 17.10 `postgres` owner login, its unselected
/// administrator role, and distinct worker/application pools selecting the roles
/// installed by the canonical 143-migration source. A random transaction-local
/// application name binds each actual backend to the administrator's activity
/// view; matching database names or OIDs alone never establish the same service.
/// All transactions are repeatable-read/read-only and are explicitly rolled back.
/// Only local session settings, catalog/journal reads and a worker zero-row
/// secret mount metadata SELECT occur; no native helper body is called.
///
/// Success describes these held observations only. It establishes no actor grant,
/// physical readiness, future connection role, or permanent pool qualification.
/// Individual adapters must continue enforcing their transaction/role/auth checks.
/// Cancellation cannot promise an awaited rollback through `Drop`.
///
/// # Errors
/// Rejects unavailable activity visibility, wrong role/service/context, an embedded
/// inventory other than exactly 1..=143, journal drift, or incompatible finite hook
/// and helper metadata/permissions. Primary verification errors survive rollback
/// errors; on success, every rollback is attempted and its first error is returned.
pub async fn verify_qualified_invocation_contract(
    admin: &PgPool,
    worker: &PgPool,
    application: &PgPool,
) -> Result<(), sqlx::Error> {
    if std::ptr::eq(admin.options(), worker.options())
        || std::ptr::eq(admin.options(), application.options())
        || std::ptr::eq(worker.options(), application.options())
    {
        return Err(invalid(
            "distinct administrator, worker and application pools required",
        ));
    }
    journal::validate_inventory()?;
    let mut admin = admin.begin().await?;
    let mut worker = match worker.begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            let _rollback = admin.rollback().await;
            return Err(error);
        }
    };
    let mut application = match application.begin().await {
        Ok(transaction) => transaction,
        Err(error) => {
            let _worker_rollback = worker.rollback().await;
            let _admin_rollback = admin.rollback().await;
            return Err(error);
        }
    };
    let verified = verify_held(&mut admin, &mut worker, &mut application).await;
    let application_rollback = application.rollback().await;
    let worker_rollback = worker.rollback().await;
    let admin_rollback = admin.rollback().await;
    verified?;
    application_rollback?;
    worker_rollback?;
    admin_rollback
}

async fn verify_held(
    admin: &mut Transaction<'_, Postgres>,
    worker: &mut Transaction<'_, Postgres>,
    application: &mut Transaction<'_, Postgres>,
) -> Result<(), sqlx::Error> {
    let owner = sessions::observe(admin, "postgres", "none").await?;
    let worker_context =
        sessions::observe(worker, "hephaestus_worker", "hephaestus_worker").await?;
    let app_context = sessions::observe(application, "hephaestus_app", "hephaestus_app").await?;
    sessions::same_service(admin, [&owner, &worker_context, &app_context]).await?;
    journal::verify(admin).await?;
    routines::verify_hooks(admin).await?;
    routines::verify_permissions(admin, worker, application).await?;
    secret_mounts::verify(admin, worker, application).await
}

fn invalid(message: &str) -> sqlx::Error {
    sqlx::Error::Protocol(format!("qualified Invocation startup: {message}"))
}
