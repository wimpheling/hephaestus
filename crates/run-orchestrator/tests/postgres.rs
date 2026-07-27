//! Opt-in `PostgreSQL` integration coverage for durable run persistence.

use run_domain::{CancelRun, RunState, StartRun};
use run_orchestrator::{
    LIFECYCLE_EVENT_SUBJECT, NatsOutboxPublisher, PgRunRepository, RepositoryError, RunRepository,
};
use runtime_types::{AgentId, CommandId, RunId};
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc, time::Duration};

#[tokio::test]
#[serial_test::serial]
// The single sequential scenario proves inbox and outbox effects in one
// database fixture; splitting it would obscure the duplicate-delivery chain.
#[allow(clippy::too_many_lines)]
async fn commands_transitions_and_outbox_are_idempotent() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect to Postgres integration database");
    let repository = PgRunRepository::new(pool.clone());
    repository.initialize().await.expect("runtime migrations");

    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        agent_id: AgentId::new(),
    };
    let created = repository.create_run(&command).await.expect("create run");
    assert!(created.created);
    assert_eq!(created.run.state, RunState::Queued);
    let duplicate = repository
        .create_run(&command)
        .await
        .expect("duplicate start command");
    assert!(!duplicate.created);
    assert_eq!(duplicate.run.id, created.run.id);

    repository
        .transition(command.run_id, RunState::LeasingVolume, None, None)
        .await
        .expect("valid transition");
    assert!(matches!(
        repository
            .transition(command.run_id, RunState::Running, None, None)
            .await,
        Err(RepositoryError::InvalidTransition(_))
    ));
    repository
        .transition(
            command.run_id,
            RunState::Failed,
            None,
            Some("deliberate test failure"),
        )
        .await
        .expect("terminal transition");
    repository
        .transition(command.run_id, RunState::CleaningUp, None, None)
        .await
        .expect("cleanup transition");
    let cleaned = repository
        .transition(command.run_id, RunState::CleanedUp, None, None)
        .await
        .expect("cleaned transition");
    assert_eq!(cleaned.state, RunState::CleanedUp);
    assert_eq!(cleaned.failure.as_deref(), Some("deliberate test failure"));

    let cancel = CancelRun {
        command_id: CommandId::new(),
        run_id: command.run_id,
        reason: String::from("duplicate test"),
    };
    assert!(
        repository
            .request_cancel(&cancel)
            .await
            .expect("first cancel")
    );
    assert!(
        !repository
            .request_cancel(&cancel)
            .await
            .expect("duplicate cancel")
    );
    let outbox = repository
        .unpublished_outbox(100)
        .await
        .expect("load outbox");
    assert_eq!(outbox.len(), 5);

    sqlx::query("DELETE FROM outbox WHERE aggregate_id = $1")
        .bind(command.run_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean outbox fixtures");
    sqlx::query("DELETE FROM run_events WHERE run_id = $1")
        .bind(command.run_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean event fixtures");
    sqlx::query("DELETE FROM command_inbox WHERE command_id IN ($1, $2)")
        .bind(command.command_id.as_uuid())
        .bind(cancel.command_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean inbox fixtures");
    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(command.run_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean run fixture");
    sqlx::query("DELETE FROM agents WHERE id = $1")
        .bind(command.agent_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean agent fixture");
}

#[tokio::test]
#[serial_test::serial]
// Publication, simulated acknowledgement loss, retry, and cleanup form one
// ordered external-system scenario.
#[allow(clippy::too_many_lines)]
async fn outbox_retries_are_deduplicated_by_jetstream_message_id() {
    let (Ok(database_url), Ok(nats_url)) = (
        env::var("HEPHAESTUS_POSTGRES_TEST_URL"),
        env::var("HEPHAESTUS_NATS_TEST_URL"),
    ) else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect to Postgres integration database");
    let repository = Arc::new(PgRunRepository::new(pool.clone()));
    repository.initialize().await.expect("runtime migrations");
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        agent_id: AgentId::new(),
    };
    repository.create_run(&command).await.expect("create run");
    let event_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM outbox WHERE aggregate_id = $1")
        .bind(command.run_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("queued outbox event");

    let client = async_nats::connect(&nats_url)
        .await
        .expect("connect to NATS integration server");
    let context = async_nats::jetstream::new(client);
    let stream_name = format!("HEPH_PHASE1B_TEST_{}", command.run_id.as_uuid().simple());
    let mut stream = context
        .create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![LIFECYCLE_EVENT_SUBJECT.to_owned()],
            duplicate_window: Duration::from_secs(60),
            ..Default::default()
        })
        .await
        .expect("create isolated event stream");
    let repository_trait: Arc<dyn RunRepository> = repository.clone();
    let publisher = NatsOutboxPublisher::new(context.clone());
    assert_eq!(
        publisher
            .publish_pending(&repository_trait, 10)
            .await
            .expect("first outbox publication"),
        1
    );
    assert_eq!(
        stream
            .info()
            .await
            .expect("first stream state")
            .state
            .messages,
        1
    );

    sqlx::query("UPDATE outbox SET published_at = NULL WHERE id = $1")
        .bind(event_id)
        .execute(&pool)
        .await
        .expect("simulate lost publication acknowledgement");
    assert_eq!(
        publisher
            .publish_pending(&repository_trait, 10)
            .await
            .expect("retried outbox publication"),
        1
    );
    assert_eq!(
        stream
            .info()
            .await
            .expect("deduplicated stream state")
            .state
            .messages,
        1,
        "JetStream stored a duplicate outbox event"
    );

    context
        .delete_stream(&stream_name)
        .await
        .expect("delete isolated event stream");
    sqlx::query("DELETE FROM outbox WHERE aggregate_id = $1")
        .bind(command.run_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean outbox fixture");
    sqlx::query("DELETE FROM run_events WHERE run_id = $1")
        .bind(command.run_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean event fixture");
    sqlx::query("DELETE FROM command_inbox WHERE command_id = $1")
        .bind(command.command_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean inbox fixture");
    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(command.run_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean run fixture");
    sqlx::query("DELETE FROM agents WHERE id = $1")
        .bind(command.agent_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean agent fixture");
}
