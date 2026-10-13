use mailbox_domain::MailboxEvent;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{MailboxPersistenceError, errors::storage};

pub async fn lock(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    event: &MailboxEvent,
) -> Result<(), MailboxPersistenceError> {
    // This unlocked lookup identifies the parent without taking a child lock.
    // Revalidate the complete owner tuple after both locks have been acquired.
    let owner: Option<(Uuid, Uuid)> =
        sqlx::query_as("SELECT project_id, instance_id FROM mailboxes WHERE id = $1")
            .bind(event.mailbox_id.as_uuid())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(storage)?;
    if owner != Some((project_id, event.instance_id.as_uuid())) {
        return Err(MailboxPersistenceError::Unavailable);
    }
    let instance: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM agent_instances WHERE id = $1 AND project_id = $2 FOR UPDATE",
    )
    .bind(event.instance_id.as_uuid())
    .bind(project_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)?;
    if instance.is_none() {
        return Err(MailboxPersistenceError::Unavailable);
    }
    let mailbox: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT project_id, instance_id, state FROM mailboxes WHERE id = $1 FOR UPDATE",
    )
    .bind(event.mailbox_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)?;
    if !mailbox.is_some_and(|(project, instance, state)| {
        project == project_id && instance == event.instance_id.as_uuid() && state == "active"
    }) {
        return Err(MailboxPersistenceError::Unavailable);
    }
    // The event trigger still checks permanent removal and dispatch support
    // against the locked instance. This helper does not grant admission.
    Ok(())
}
