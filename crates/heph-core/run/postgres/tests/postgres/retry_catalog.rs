use super::support::{
    retry_command, seed_instance, seed_mailbox_input, seed_retry_request, seed_run_request,
};
use run_domain::{RunKind, StartRun};
use run_orchestrator::{RunRepository, RunRuntimeCatalog};
use run_postgres::PgRunRepository;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use sqlx::postgres::PgPoolOptions;
use std::env;

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
    sqlx::migrate!("../../../../migrations")
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
