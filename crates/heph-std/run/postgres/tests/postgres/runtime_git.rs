use super::support::{seed_instance, seed_run_request};
use run_domain::{RunKind, StartRun};
use run_orchestrator::{RepositoryError, RunRepository, RunRuntimeCatalog};
use run_postgres::PgRunRepository;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use sqlx::postgres::PgPoolOptions;
use std::env;

#[tokio::test]
#[serial_test::serial]
async fn runtime_git_context_uses_immutable_target_and_fails_closed_without_it() {
    let Ok(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect to Postgres integration database");
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
        requires_state: false,
    };
    seed_instance(&pool, &command).await;
    sqlx::query(
        "INSERT INTO release_artifacts
         (id, release_id, path, kind, mode, content_hash, size_bytes,
          media_type, storage_key)
         VALUES ($1, $2, 'bin/agent', 'executable', 365, $3, 5,
                 'application/octet-stream', $4)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(command.release_id.as_uuid())
    .bind([7_u8; 32].as_slice())
    .bind(uuid::Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("runtime artifact");
    let (project_id, trigger_repository) = seed_run_request(&pool, &command).await;
    let repository = PgRunRepository::new(pool.clone());
    let created = repository.create_run(&command).await.expect("create run");
    let trigger_context = repository
        .load_runtime(&created.run)
        .await
        .expect("load trigger context before runtime Git authority");
    assert_eq!(trigger_context.repository_id, Some(trigger_repository));
    // This fixture bypasses immutable publication/FK triggers only to model
    // already-persisted authority rows without recreating the issuer graph.
    sqlx::query("SET session_replication_role = 'replica'")
        .execute(&pool)
        .await
        .expect("enable isolated runtime Git fixture mode");

    let capability_repository = uuid::Uuid::new_v4();
    let capability_binding = uuid::Uuid::new_v4();
    sqlx::query(
        "UPDATE agent_instance_revisions
            SET publication_mode = 'runtime_git',
                publication_repository_binding_id = $2
          WHERE id = $1",
    )
    .bind(command.instance_revision_id.as_uuid())
    .bind(capability_binding)
    .execute(&pool)
    .await
    .expect("select immutable runtime Git publication mode");
    assert!(matches!(
        repository.ensure_runtime_git_provenance(&created.run).await,
        Err(RepositoryError::InvalidData(
            "runtime Git publication repository capability"
        ))
    ));

    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, $3)",
    )
    .bind(capability_repository)
    .bind(project_id)
    .bind(format!("runtime-target-{capability_repository}"))
    .execute(&pool)
    .await
    .expect("capability repository");
    let capability_requirement = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_capability_bindings
         (id, instance_revision_id, release_agent_id, requirement_id,
          requirement_hash, slot_key, resource_kind, resource_id,
          granted_operations, normalized_hash, authorization_model_version,
          created_by)
         VALUES ($1, $2, $3, $4, $5, 'session', 'repository', $6,
                 ARRAY['git_read', 'update_ref'], $7, 'test/v1', $8)",
    )
    .bind(capability_binding)
    .bind(command.instance_revision_id.as_uuid())
    .bind(command.release_agent_id.as_uuid())
    .bind(capability_requirement)
    .bind([8_u8; 32].as_slice())
    .bind(capability_repository)
    .bind([9_u8; 32].as_slice())
    .bind(uuid::Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("runtime Git capability binding");
    sqlx::query(
        "INSERT INTO agent_git_capability_bindings
         (binding_id, instance_revision_id, requirement_id, grammar_version,
          git_operations, ref_globs, changed_path_globs,
          branch_update_policy, branch_create, branch_delete, tag_create,
          tag_update, tag_delete, other_create, other_update, other_delete,
          request_bytes, pack_bytes, object_count, ref_updates,
          exact_parent_required, normalized_hash)
         VALUES ($1, $2, $3, 1, ARRAY['discover', 'fetch', 'receive'],
                 ARRAY['refs/heads/main'], ARRAY['.heph/session/**'],
                 'fast_forward_only', false, false, false, false, false,
                 false, false, false, 1048576, 8388608, 10000, 8, true, $4)",
    )
    .bind(capability_binding)
    .bind(command.instance_revision_id.as_uuid())
    .bind(capability_requirement)
    .bind([10_u8; 32].as_slice())
    .execute(&pool)
    .await
    .expect("runtime Git typed capability binding");
    let target_commit = "a".repeat(40);
    sqlx::query("SET session_replication_role = 'origin'")
        .execute(&pool)
        .await
        .expect("restore database trigger behavior");

    repository
        .ensure_runtime_git_provenance(&created.run)
        .await
        .expect("ensure immutable runtime target provenance");
    repository
        .ensure_runtime_git_provenance(&created.run)
        .await
        .expect("replay immutable runtime target provenance");

    let runtime_context = repository
        .load_runtime(&created.run)
        .await
        .expect("load immutable runtime Git context");
    assert_eq!(runtime_context.repository_id, Some(capability_repository));
    assert_eq!(runtime_context.git_ref.as_deref(), Some("refs/heads/main"));
    assert_eq!(
        runtime_context.commit_sha.as_deref(),
        Some(target_commit.as_str())
    );

    sqlx::query("SET session_replication_role = 'replica'")
        .execute(&pool)
        .await
        .expect("enable isolated provenance conflict fixture mode");
    sqlx::query(
        "UPDATE run_instance_provenance
            SET target_repository_id = $2
          WHERE run_id = $1",
    )
    .bind(command.run_id.as_uuid())
    .bind(trigger_repository)
    .execute(&pool)
    .await
    .expect("create provenance conflict fixture");
    sqlx::query("SET session_replication_role = 'origin'")
        .execute(&pool)
        .await
        .expect("restore provenance conflict fixture mode");
    assert!(matches!(
        repository.ensure_runtime_git_provenance(&created.run).await,
        Err(RepositoryError::InvalidData(
            "runtime Git provenance conflict"
        ))
    ));
    sqlx::query("SET session_replication_role = 'replica'")
        .execute(&pool)
        .await
        .expect("enable provenance cleanup fixture mode");
    sqlx::query("DELETE FROM run_instance_provenance WHERE run_id = $1")
        .bind(command.run_id.as_uuid())
        .execute(&pool)
        .await
        .expect("remove provenance fixture");
    sqlx::query("SET session_replication_role = 'origin'")
        .execute(&pool)
        .await
        .expect("restore provenance cleanup fixture mode");
    assert!(matches!(
        repository.load_runtime(&created.run).await,
        Err(run_orchestrator::RunRuntimeCatalogError::InvalidData(
            "normal run target provenance"
        ))
    ));
}
