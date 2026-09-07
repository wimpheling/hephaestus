//! Opt-in `PostgreSQL` integration coverage for durable run persistence.

use run_domain::{CancelRun, RunKind, RunState, StartRun};
use run_orchestrator::{RepositoryError, RunRepository, RunRuntimeArtifactKind, RunRuntimeCatalog};
use run_postgres::PgRunRepository;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use sha2::{Digest, Sha256};
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
    sqlx::migrate!("../../migrations")
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

#[tokio::test]
#[serial_test::serial]
async fn retry_runtime_catalog_preserves_mailbox_input_and_rejects_broken_lineage() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("connect to Postgres integration database");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("runtime migrations");
    let repository = PgRunRepository::new(pool.clone());

    let source = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: false,
    };
    seed_instance(&pool, &source).await;
    let (_, repository_id) = seed_run_request(&pool, &source).await;
    let source_created = repository
        .create_run(&source)
        .await
        .expect("mailbox source run");
    let body = b"exact accepted retry body".to_vec();
    seed_mailbox_input(&pool, &source, &body).await;
    let source_runtime = repository
        .load_runtime(&source_created.run)
        .await
        .expect("load mailbox source runtime");
    assert_eq!(source_runtime.mailbox_event.as_ref().unwrap().body, body);

    let retry = retry_command(&source);
    seed_retry_request(&pool, &retry, repository_id, source.run_id).await;
    let retry_created = repository.create_run(&retry).await.expect("retry run");
    let retry_runtime = repository
        .load_runtime(&retry_created.run)
        .await
        .expect("retry retains accepted mailbox input");
    assert_eq!(
        retry_runtime.mailbox_event.as_ref().unwrap().body,
        b"exact accepted retry body"
    );

    let retry_again = retry_command(&retry);
    seed_retry_request(&pool, &retry_again, repository_id, retry.run_id).await;
    let retry_again_created = repository
        .create_run(&retry_again)
        .await
        .expect("retry of retry run");
    let retry_again_runtime = repository
        .load_runtime(&retry_again_created.run)
        .await
        .expect("retry of retry retains accepted mailbox input");
    assert_eq!(
        retry_again_runtime.mailbox_event.as_ref().unwrap().body,
        b"exact accepted retry body"
    );

    // A non-mailbox Git-trigger retry remains a valid runtime with no event.
    let plain = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: false,
    };
    seed_instance(&pool, &plain).await;
    let (_, plain_repository) = seed_run_request(&pool, &plain).await;
    repository
        .create_run(&plain)
        .await
        .expect("plain source run");
    let plain_retry = retry_command(&plain);
    seed_retry_request(&pool, &plain_retry, plain_repository, plain.run_id).await;
    let plain_retry_created = repository
        .create_run(&plain_retry)
        .await
        .expect("plain retry run");
    assert!(
        repository
            .load_runtime(&plain_retry_created.run)
            .await
            .expect("plain retry runtime")
            .mailbox_event
            .is_none()
    );

    // A retry whose parent request disappeared fails closed instead of
    // silently becoming a fresh non-mailbox execution.
    let orphan_parent = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: false,
    };
    seed_instance(&pool, &orphan_parent).await;
    let (_, orphan_repository) = seed_run_request(&pool, &orphan_parent).await;
    repository
        .create_run(&orphan_parent)
        .await
        .expect("orphan parent run");
    let orphan_retry = retry_command(&orphan_parent);
    seed_retry_request(
        &pool,
        &orphan_retry,
        orphan_repository,
        orphan_parent.run_id,
    )
    .await;
    let orphan_retry_created = repository
        .create_run(&orphan_retry)
        .await
        .expect("orphan retry run");
    sqlx::query("DELETE FROM run_requests WHERE run_id = $1")
        .bind(orphan_parent.run_id.as_uuid())
        .execute(&pool)
        .await
        .expect("remove orphan parent request");
    let error = repository
        .load_runtime(&orphan_retry_created.run)
        .await
        .expect_err("orphan retry lineage must fail closed");
    assert!(matches!(
        error,
        run_orchestrator::RunRuntimeCatalogError::InvalidData("retry run lineage is invalid")
    ));

    // Reinsert the parent request as a retry of its child to form a cycle;
    // the recursive catalog query must reject it without looping.
    seed_retry_request(
        &pool,
        &orphan_parent,
        orphan_repository,
        orphan_retry.run_id,
    )
    .await;
    let cycle_error = repository
        .load_runtime(&orphan_retry_created.run)
        .await
        .expect_err("cyclic retry lineage must fail closed");
    assert!(matches!(
        cycle_error,
        run_orchestrator::RunRuntimeCatalogError::InvalidData("retry run lineage is invalid")
    ));

    // A long but acyclic chain is bounded as well. It must report invalid
    // provenance instead of silently dropping the accepted input at depth 64.
    let deep_source = StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: false,
    };
    seed_instance(&pool, &deep_source).await;
    let (_, deep_repository) = seed_run_request(&pool, &deep_source).await;
    repository
        .create_run(&deep_source)
        .await
        .expect("deep source run");
    let mut deep_parent = deep_source;
    let mut deep_created = None;
    for _ in 0..65 {
        let child = retry_command(&deep_parent);
        seed_retry_request(&pool, &child, deep_repository, deep_parent.run_id).await;
        let created = repository.create_run(&child).await.expect("deep retry run");
        deep_parent = child;
        deep_created = Some(created.run);
    }
    let depth_error = repository
        .load_runtime(&deep_created.expect("deep retry run"))
        .await
        .expect_err("exhausted retry lineage must fail closed");
    assert!(matches!(
        depth_error,
        run_orchestrator::RunRuntimeCatalogError::InvalidData("retry run lineage is invalid")
    ));
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

async fn seed_mailbox_input(pool: &sqlx::PgPool, command: &StartRun, body: &[u8]) {
    let (project_id, _) = seed_run_request_scope(pool, command).await;
    let mailbox_id = uuid::Uuid::new_v4();
    let payload_id = uuid::Uuid::new_v4();
    let event_id = uuid::Uuid::new_v4();
    let integrity_hash: [u8; 32] = Sha256::digest(body).into();
    sqlx::query(
        "INSERT INTO mailboxes (id, project_id, instance_id, state)
         VALUES ($1, $2, $3, 'active')",
    )
    .bind(mailbox_id)
    .bind(project_id)
    .bind(command.instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("mailbox input mailbox");
    sqlx::query(
        "INSERT INTO mailbox_payloads
         (id, mailbox_id, project_id, encoded_body, encoded_length,
          decoded_length, integrity_hash)
         VALUES ($1, $2, $3, $4, $5, $5, $6)",
    )
    .bind(payload_id)
    .bind(mailbox_id)
    .bind(project_id)
    .bind(body)
    .bind(i32::try_from(body.len()).expect("bounded test body"))
    .bind(integrity_hash.as_slice())
    .execute(pool)
    .await
    .expect("mailbox input payload");
    sqlx::query(
        "INSERT INTO mailbox_events
         (id, mailbox_id, project_id, instance_id, body_id, producer_kind,
          producer_id, deduplication_scope, deduplication_key, method, route,
          received_at)
         VALUES ($1, $2, $3, $4, $5, 'gateway', 'retry-test', 'retry-test',
                 $6, 'POST', '/retry-test', now())",
    )
    .bind(event_id)
    .bind(mailbox_id)
    .bind(project_id)
    .bind(command.instance_id.as_uuid())
    .bind(payload_id)
    .bind(format!("event-{event_id}"))
    .execute(pool)
    .await
    .expect("mailbox input event");
    sqlx::query(
        "INSERT INTO mailbox_delivery_attempts
         (id, event_id, mailbox_id, attempt_number, state, command_id,
          instance_id, instance_revision_id, run_id, target_ref, target_commit,
          state_access_outcome)
         VALUES ($1, $2, $3, 1, 'leased', $4, $5, $6, $7,
                 'refs/heads/main', $8, 'no_state')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(event_id)
    .bind(mailbox_id)
    .bind(uuid::Uuid::new_v4())
    .bind(command.instance_id.as_uuid())
    .bind(command.instance_revision_id.as_uuid())
    .bind(command.run_id.as_uuid())
    .bind("a".repeat(40))
    .execute(pool)
    .await
    .expect("mailbox input attempt");
}

async fn seed_run_request_scope(
    pool: &sqlx::PgPool,
    command: &StartRun,
) -> (uuid::Uuid, uuid::Uuid) {
    sqlx::query_as(
        "SELECT instance.project_id, attachment.repository_id
           FROM agent_instances instance
           JOIN agent_attachments attachment ON attachment.instance_id = instance.id
          WHERE instance.id = $1 AND attachment.id = $2",
    )
    .bind(command.instance_id.as_uuid())
    .bind(command.attachment_id.expect("run attachment").as_uuid())
    .fetch_one(pool)
    .await
    .expect("run request scope")
}

fn retry_command(source: &StartRun) -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: source.instance_id,
        instance_revision_id: source.instance_revision_id,
        release_id: source.release_id,
        release_agent_id: source.release_agent_id,
        attachment_id: source.attachment_id,
        kind: RunKind::Normal,
        requires_state: source.requires_state,
    }
}

async fn seed_retry_request(
    pool: &sqlx::PgPool,
    command: &StartRun,
    repository_id: uuid::Uuid,
    source_run_id: RunId,
) {
    sqlx::query(
        "INSERT INTO run_requests
         (id, repository_id, commit_sha, git_ref, run_id, command_id,
          trigger_command_id, instance_id, instance_revision_id, release_id,
          release_agent_id, attachment_id, platform_policy_version,
          request_kind, requires_state, retry_of_run_id, attempt)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, $5, $6, $7, $8,
                 $9, $10, 'test/v1', 'instance_normal', $11, $12, $13)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind(command.run_id.as_uuid())
    .bind(command.command_id.as_uuid())
    .bind(command.instance_id.as_uuid())
    .bind(command.instance_revision_id.as_uuid())
    .bind(command.release_id.as_uuid())
    .bind(command.release_agent_id.as_uuid())
    .bind(command.attachment_id.expect("retry attachment").as_uuid())
    .bind(command.requires_state)
    .bind(source_run_id.as_uuid())
    .bind(2_i32)
    .execute(pool)
    .await
    .expect("seed retry run request");
}

async fn seed_run_request(pool: &sqlx::PgPool, command: &StartRun) -> (uuid::Uuid, uuid::Uuid) {
    let (project_id, repository_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT instance.project_id, attachment.repository_id
         FROM agent_instances instance
         JOIN agent_attachments attachment ON attachment.instance_id = instance.id
         WHERE instance.id = $1 AND attachment.id = $2",
    )
    .bind(command.instance_id.as_uuid())
    .bind(
        command
            .attachment_id
            .expect("normal run attachment")
            .as_uuid(),
    )
    .fetch_one(pool)
    .await
    .expect("load run request scope fixture");
    sqlx::query(
        "INSERT INTO run_requests
         (id, repository_id, commit_sha, git_ref, run_id, command_id,
          trigger_command_id, instance_id, instance_revision_id, release_id,
          release_agent_id, attachment_id, platform_policy_version,
          request_kind, requires_state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, $5, $6, $7, $8,
                 $9, $10, 'test/v1', 'instance_normal', $11)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind(command.run_id.as_uuid())
    .bind(command.command_id.as_uuid())
    .bind(command.instance_id.as_uuid())
    .bind(command.instance_revision_id.as_uuid())
    .bind(command.release_id.as_uuid())
    .bind(command.release_agent_id.as_uuid())
    .bind(
        command
            .attachment_id
            .expect("normal run attachment")
            .as_uuid(),
    )
    .bind(command.requires_state)
    .execute(pool)
    .await
    .expect("seed run request before run");
    (project_id, repository_id)
}
