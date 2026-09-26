use super::support::{seed_instance, seed_run_request};
use run_domain::{CancelRun, RunKind, RunState, StartRun};
use run_orchestrator::{RepositoryError, RunRepository, RunRuntimeArtifactKind, RunRuntimeCatalog};
use run_postgres::PgRunRepository;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use sqlx::postgres::PgPoolOptions;
use std::env;

#[tokio::test]
#[serial_test::serial]
// The single sequential scenario proves inbox and durable event effects in one
// database fixture; splitting it would obscure the duplicate-delivery chain.
#[allow(clippy::too_many_lines)]
async fn commands_transitions_and_events_are_idempotent() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect to Postgres integration database");
    let repository = PgRunRepository::new(pool.clone());
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("runtime migrations");
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
    let runtime_storage_key = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO release_artifacts
         (id, release_id, path, kind, mode, content_hash, size_bytes,
          media_type, storage_key)
         VALUES ($1, $2, 'config/runtime.json', 'file', 292, $3, 17,
                 'application/json', $4)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(command.release_id.as_uuid())
    .bind([7_u8; 32].as_slice())
    .bind(runtime_storage_key)
    .execute(&pool)
    .await
    .expect("runtime artifact");
    let (project_id, repository_id) = seed_run_request(&pool, &command).await;
    let created = repository.create_run(&command).await.expect("create run");
    assert!(created.created);
    assert_eq!(created.run.state, RunState::Queued);
    let runtime = repository
        .load_runtime(&created.run)
        .await
        .expect("load exact runtime provenance");
    assert_eq!(runtime.parameters, serde_json::json!({}));
    assert_eq!(runtime.repository_id, Some(repository_id));
    assert_eq!(runtime.git_ref.as_deref(), Some("refs/heads/main"));
    assert_eq!(
        runtime.commit_sha.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(runtime.artifacts.len(), 1);
    assert_eq!(runtime.artifacts[0].path, "config/runtime.json");
    assert_eq!(runtime.artifacts[0].kind, RunRuntimeArtifactKind::File);
    assert_eq!(runtime.artifacts[0].mode, 0o444);
    assert_eq!(runtime.artifacts[0].content_hash, [7; 32]);
    assert_eq!(runtime.artifacts[0].size_bytes, 17);
    assert_eq!(runtime.artifacts[0].storage_key, runtime_storage_key);
    assert!(runtime.previous_artifacts.is_empty());
    assert!(
        repository
            .run_is_live(command.run_id)
            .await
            .expect("load live runtime ownership")
    );
    let scoped_events: Vec<(String, uuid::Uuid, Option<uuid::Uuid>, Option<uuid::Uuid>)> =
        sqlx::query_as(
            "SELECT scope_kind, occurrence_id, related_id_one, related_id_two
         FROM application_events
         WHERE aggregate_type = 'run' AND aggregate_id = $1
           AND change_kind = 'created'
           AND occurrence_id = (
               SELECT occurrence_id FROM application_events
               WHERE aggregate_type = 'run' AND aggregate_id = $1
                 AND scope_kind = 'project' AND change_kind = 'created'
               ORDER BY cursor DESC LIMIT 1
           )
         ORDER BY scope_kind",
        )
        .bind(command.run_id.as_uuid())
        .fetch_all(&pool)
        .await
        .expect("load multi-scope run events");
    assert_eq!(scoped_events.len(), 3);
    assert_eq!(
        scoped_events
            .iter()
            .map(|(scope, _, _, _)| scope.as_str())
            .collect::<Vec<_>>(),
        ["agent_instance", "project", "run"]
    );
    assert!(
        scoped_events
            .iter()
            .all(|(_, occurrence, related_project, related_repository)| {
                *occurrence == scoped_events[0].1
                    && *related_project == Some(project_id)
                    && *related_repository == Some(repository_id)
            })
    );
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
    let transition_events: Vec<(String, uuid::Uuid)> = sqlx::query_as(
        "SELECT scope_kind, occurrence_id
         FROM application_events
         WHERE aggregate_type = 'run' AND aggregate_id = $1
           AND change_kind = 'state_changed' AND safe_state = 'running'
         ORDER BY scope_kind",
    )
    .bind(command.run_id.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("load multi-scope transition events");
    assert_eq!(transition_events.len(), 3);
    assert_eq!(
        transition_events
            .iter()
            .map(|(scope, _)| scope.as_str())
            .collect::<Vec<_>>(),
        ["agent_instance", "project", "run"]
    );
    assert!(
        transition_events
            .iter()
            .all(|(_, occurrence)| *occurrence == transition_events[0].1)
    );
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
    assert!(
        !repository
            .run_is_live(command.run_id)
            .await
            .expect("load cleaned runtime ownership")
    );

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
    let event_count: i64 = sqlx::query_scalar("SELECT count(*) FROM run_events WHERE run_id = $1")
        .bind(command.run_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("load durable run events");
    assert_eq!(event_count, 5);
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
