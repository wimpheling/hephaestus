//! Shared request-context setup for UI browser projections.

use identity_domain::RequestId;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(super) async fn set_verified_actor_context(
    transaction: &mut Transaction<'_, Postgres>,
    actor_id: Uuid,
    request_id: RequestId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1::text, true),
                set_config('hephaestus.subject_type', 'user', true),
                set_config('hephaestus.request_id', $2::text, true),
                set_config('hephaestus.occurrence_id', $2::text, true)",
    )
    .bind(actor_id)
    .bind(request_id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map(|_| ())
}
