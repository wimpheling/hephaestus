//! Opt-in `PostgreSQL` integration coverage for durable run persistence.

use run_domain::{CancelRun, RunKind, RunState, StartRun};
use run_orchestrator::{
    LIFECYCLE_EVENT_SUBJECT, NatsOutboxPublisher, PgRunRepository, RepositoryError, RunRepository,
};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
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
    sqlx::query("DELETE FROM outbox WHERE aggregate_type = 'run'")
        .execute(&pool)
        .await
        .expect("isolate run outbox fixture");

    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: true,
    };
    seed_instance(&pool, &command).await;
    let created = repository.create_run(&command).await.expect("create run");
    assert!(created.created);
    assert_eq!(created.run.state, RunState::Queued);
    let duplicate = repository
        .create_run(&command)
        .await
        .expect("duplicate start command");
    assert!(!duplicate.created);
    assert_eq!(duplicate.run.id, created.run.id);

    let update = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: None,
        kind: RunKind::Update,
        requires_state: true,
    };
    seed_instance(&pool, &update).await;
    sqlx::query(
        "INSERT INTO runs
         (id, instance_id, instance_revision_id, release_id, release_agent_id,
          run_kind, command_id, state, requires_state, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, 'update', $6, 'queued', true, now(), now())",
    )
    .bind(update.run_id.as_uuid())
    .bind(update.instance_id.as_uuid())
    .bind(update.instance_revision_id.as_uuid())
    .bind(update.release_id.as_uuid())
    .bind(update.release_agent_id.as_uuid())
    .bind(update.command_id.as_uuid())
    .execute(&pool)
    .await
    .expect("precreate update run");
    let adopted = repository
        .create_run(&update)
        .await
        .expect("adopt precreated update run");
    assert!(adopted.created);
    assert_eq!(adopted.run, repository.get(update.run_id).await.unwrap());

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
    sqlx::query("DELETE FROM command_inbox WHERE command_id = $1")
        .bind(update.command_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean update inbox");
    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(update.run_id.as_uuid())
        .execute(&pool)
        .await
        .expect("clean update run");
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
    sqlx::query("DELETE FROM outbox WHERE aggregate_type = 'run'")
        .execute(&pool)
        .await
        .expect("isolate run outbox fixture");
    let command = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: true,
    };
    seed_instance(&pool, &command).await;
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
    // Other workspace integration tests exercise the production topology on
    // the same disposable server. Remove that event stream so this test can
    // install a stream with an isolated duplicate window.
    drop(context.delete_stream("HEPH_RUN_EVENTS").await);
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
}

// Keeping the complete exact-provenance graph together makes this fixture auditable.
#[allow(clippy::too_many_lines)]
async fn seed_instance(pool: &sqlx::PgPool, command: &StartRun) {
    let organization_id = uuid::Uuid::new_v4();
    let project_id = uuid::Uuid::new_v4();
    let repository_id = uuid::Uuid::new_v4();
    let family_id = uuid::Uuid::new_v4();
    let build_request_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("runtime-{organization_id}"))
        .execute(pool)
        .await
        .expect("runtime organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("runtime-{project_id}"))
        .execute(pool)
        .await
        .expect("runtime project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, $3)",
    )
    .bind(repository_id)
    .bind(project_id)
    .bind(format!("runtime-{repository_id}"))
    .execute(pool)
    .await
    .expect("runtime repository");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'runtime')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("runtime family");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state, completed_at)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())",
    )
    .bind(build_request_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("runtime build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8,
                 'published', now())",
    )
    .bind(command.release_id.as_uuid())
    .bind(repository_id)
    .bind(format!("test-{}", command.release_id))
    .bind("a".repeat(40))
    .bind(build_request_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("runtime release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, 'runtime', 'Runtime', '{}', $4, $5)",
    )
    .bind(command.release_agent_id.as_uuid())
    .bind(command.release_id.as_uuid())
    .bind(family_id)
    .bind([4_u8; 32].as_slice())
    .bind(command.requires_state)
    .execute(pool)
    .await
    .expect("runtime release agent");
    sqlx::query(
        "INSERT INTO agent_instances (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(command.instance_id.as_uuid())
    .bind(project_id)
    .bind(family_id)
    .bind(format!("runtime-{}", command.instance_id))
    .execute(pool)
    .await
    .expect("runtime instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5, 'test/v1', true)",
    )
    .bind(command.instance_revision_id.as_uuid())
    .bind(command.instance_id.as_uuid())
    .bind(command.release_agent_id.as_uuid())
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("runtime revision");
    sqlx::query(
        "UPDATE agent_instances SET active_revision_id = $2
         WHERE id = $1",
    )
    .bind(command.instance_id.as_uuid())
    .bind(command.instance_revision_id.as_uuid())
    .execute(pool)
    .await
    .expect("activate runtime revision");
    if let Some(attachment_id) = command.attachment_id {
        sqlx::query(
            "INSERT INTO agent_attachments
             (id, instance_id, project_id, repository_id, ref_selector,
              trigger_policy)
             VALUES ($1, $2, $3, $4, 'refs/heads/main', 'manual')",
        )
        .bind(attachment_id.as_uuid())
        .bind(command.instance_id.as_uuid())
        .bind(project_id)
        .bind(repository_id)
        .execute(pool)
        .await
        .expect("runtime attachment");
    }
}
