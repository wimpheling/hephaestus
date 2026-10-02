use mailbox_domain::{MailboxEvent, MailboxId};
use mailbox_postgres::{AcceptedMailboxEvent, MailboxPersistenceError, PostgresMailboxRepository};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{env, sync::Arc, time::Duration};
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::support::{Fixture, seed_instance};

pub struct Harness {
    pub pool: PgPool,
    pub fixture: Fixture,
    pub store: Arc<PostgresMailboxRepository>,
    pub mailbox: MailboxId,
}

impl Harness {
    pub async fn connect() -> Option<Self> {
        let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
        let pool = PgPoolOptions::new()
            .max_connections(6)
            .connect(&database_url)
            .await
            .expect("connect real PostgreSQL");
        sqlx::migrate!("../../../../../migrations")
            .run(&pool)
            .await
            .expect("apply mailbox migrations");
        let fixture = seed_instance(&pool).await;
        let store = Arc::new(PostgresMailboxRepository::new(pool.clone()));
        let mailbox = MailboxId::new();
        store
            .ensure_mailbox(fixture.project, mailbox, fixture.instance)
            .await
            .expect("create lock-order mailbox");
        Some(Self {
            pool,
            fixture,
            store,
            mailbox,
        })
    }

    pub fn accept(
        &self,
        event: MailboxEvent,
        body: &'static [u8],
    ) -> JoinHandle<Result<AcceptedMailboxEvent, MailboxPersistenceError>> {
        let store = Arc::clone(&self.store);
        let project = self.fixture.project;
        tokio::spawn(async move {
            store
                .accept(project, &event, body, u32::try_from(body.len()).unwrap())
                .await
        })
    }

    pub async fn wait_for_acceptors(&self, count: i64) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let waiting: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity
                     WHERE datname = current_database() AND pid <> pg_backend_pid()
                       AND state = 'active' AND wait_event_type = 'Lock'
                       AND query LIKE 'SELECT id FROM agent_instances WHERE id = % FOR UPDATE'",
                )
                .fetch_one(&self.pool)
                .await
                .expect("inspect actual parent-lock wait");
                if waiting >= count {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("acceptors must wait at parent lock before any child writes");
    }

    pub async fn counts(&self) -> (i64, i64, i64, i64) {
        sqlx::query_as(
            "SELECT
                 (SELECT count(*) FROM mailbox_payloads WHERE mailbox_id = $1),
                 (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1),
                 (SELECT count(*) FROM mailbox_deliveries WHERE mailbox_id = $1),
                 (SELECT count(*) FROM outbox WHERE id IN
                      (SELECT id FROM mailbox_events WHERE mailbox_id = $1))",
        )
        .bind(self.mailbox.as_uuid())
        .fetch_one(&self.pool)
        .await
        .expect("read exact mailbox durable children")
    }

    pub async fn settle_outbox(&self, event: Uuid) {
        // These tests own persistence only; keep their wake outbox away from
        // the separate shared JetStream consumer integration fixture.
        sqlx::query("UPDATE outbox SET published_at = now() WHERE id = $1")
            .bind(event)
            .execute(&self.pool)
            .await
            .expect("isolate lock-order fixture outbox");
    }
}

pub async fn join(
    task: JoinHandle<Result<AcceptedMailboxEvent, MailboxPersistenceError>>,
) -> Result<AcceptedMailboxEvent, MailboxPersistenceError> {
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("acceptor must finish after parent lock release")
        .expect("join acceptor")
}
