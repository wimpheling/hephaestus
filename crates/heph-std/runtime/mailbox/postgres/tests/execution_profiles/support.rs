use std::borrow::Cow;

use mailbox_dispatch::MailboxDispatchCommand;
use mailbox_domain::{MailboxId, MailboxOperationIdentity};
use mailbox_postgres::PostgresMailboxRepository;
use sqlx::{PgPool, migrate::Migrator};

const MIGRATIONS: Migrator = sqlx::migrate!("../../../../../migrations");

pub async fn setup(pool: &PgPool, profiles: bool) -> MailboxDispatchCommand {
    Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|m| m.version <= 107)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
    .run(pool)
    .await
    .unwrap();
    if profiles {
        sqlx::raw_sql(include_str!(
            "../../../../../../../migrations/0108_run_volume_leases.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../../../../../../../migrations/0109_run_cleanup.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
    }
    let f = crate::fixture::seed_instance(pool).await;
    let store = PostgresMailboxRepository::new(pool.clone());
    let mailbox = MailboxId::new();
    store
        .ensure_mailbox(f.project, mailbox, f.instance)
        .await
        .unwrap();
    let body = b"explicit legacy accepted producer";
    let event = crate::fixture::event(mailbox, f.instance, body);
    let accepted = store
        .accept(f.project, &event, body, u32::try_from(body.len()).unwrap())
        .await
        .unwrap();
    sqlx::query("UPDATE mailbox_deliveries SET disposition='eligible',next_eligible_at=NULL WHERE event_id=$1")
        .bind(accepted.event_id.as_uuid()).execute(pool).await.unwrap();
    MailboxDispatchCommand {
        operation_id: MailboxOperationIdentity::dispatch(mailbox, accepted.event_id, 1).id(),
        event_id: accepted.event_id,
    }
}
