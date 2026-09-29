use super::*;

#[tokio::test]
#[serial]
async fn runtime_git_resolver_is_immutable_and_runtime_rows_are_classified() {
    let database_url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL")
        .expect("HEPHAESTUS_POSTGRES_TEST_URL is required for runtime Git resolver tests");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply migrations");

    let run_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let capability_repository = Uuid::new_v4();
    let trigger_repository = Uuid::new_v4();
    let target_commit = "a".repeat(40);
    sqlx::query("SET session_replication_role = 'replica'")
        .execute(&pool)
        .await
        .expect("enable isolated fixture mode");
    sqlx::query(
        "INSERT INTO runs
         (id, command_id, state, created_at, updated_at, instance_id,
          instance_revision_id, release_id, release_agent_id, attachment_id,
          run_kind, requires_state)
         VALUES ($1, $2, 'provisioning', now(), now(), $3, $4, $5, $6, $7,
                 'normal', false)",
    )
    .bind(run_id)
    .bind(Uuid::new_v4())
    .bind(instance_id)
    .bind(revision_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("seed run");
    sqlx::query(
        "INSERT INTO run_authorization_snapshots
         (id, run_id, instance_id, instance_revision_id,
          authorization_model_version, normalized_hash)
         VALUES ($1, $2, $3, $4, 'test/v1', $5)",
    )
    .bind(snapshot_id)
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind([1_u8; 32].as_slice())
    .execute(&pool)
    .await
    .expect("seed authorization snapshot");
    sqlx::query(
        "INSERT INTO run_git_authority_snapshots
         (snapshot_id, instance_revision_id, binding_id, repository_id,
          grammar_version, git_operations, ref_globs, changed_path_globs,
          branch_update_policy, branch_create, branch_delete, tag_create,
          tag_update, tag_delete, other_create, other_update, other_delete,
          request_bytes, pack_bytes, object_count, ref_updates,
          exact_parent_required, expected_parent, normalized_hash)
         VALUES ($1, $2, $3, $4, 1, $5, $6, ARRAY[]::text[],
                 'fast_forward_only', false, false, false, false, false,
                 false, false, false, 1, 1, 1, 1, false, NULL, $7)",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(Uuid::new_v4())
    .bind(capability_repository)
    .bind(vec![String::from("fetch")])
    .bind(vec![String::from("refs/heads/main")])
    .bind([2_u8; 32].as_slice())
    .execute(&pool)
    .await
    .expect("seed Git snapshot");
    sqlx::query(
        "INSERT INTO run_instance_provenance
         (run_id, instance_id, instance_revision_id, release_id,
          release_agent_id, attachment_id, target_repository_id, target_ref,
          target_commit, parameter_hash, platform_policy_version, phase,
          authorization_model_version)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'refs/heads/main', $8, $9,
                 'platform/test', 'normal', 'test/v1')",
    )
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(capability_repository)
    .bind(&target_commit)
    .bind([3_u8; 32].as_slice())
    .execute(&pool)
    .await
    .expect("seed immutable provenance");
    let repository = PgWorkspaceMetadataRepository::new(pool.clone());
    let request = repository
        .runtime_git_request(runtime_types::RunId::from_uuid(run_id))
        .await
        .expect("resolve immutable runtime Git request")
        .expect("runtime Git request");
    assert_eq!(
        request,
        RuntimeGitWorkspaceRequest {
            repository_id: capability_repository,
            target_repository_id: capability_repository,
            target_ref: String::from("refs/heads/main"),
            target_commit: target_commit.clone(),
            git_operations: vec![String::from("fetch")],
            ref_globs: vec![String::from("refs/heads/main")],
        }
    );
    request.validate().expect("authorized immutable target");

    sqlx::query("UPDATE run_instance_provenance SET target_repository_id = $2 WHERE run_id = $1")
        .bind(run_id)
        .bind(trigger_repository)
        .execute(&pool)
        .await
        .expect("move trigger repository");
    let cross_repository = repository
        .runtime_git_request(runtime_types::RunId::from_uuid(run_id))
        .await
        .expect("cross-repository denial")
        .expect("immutable target is retained for fail-closed validation");
    assert!(cross_repository.validate().is_err());
    sqlx::query("UPDATE run_instance_provenance SET target_repository_id = $2 WHERE run_id = $1")
        .bind(run_id)
        .bind(capability_repository)
        .execute(&pool)
        .await
        .expect("restore capability repository");
    sqlx::query("UPDATE run_git_authority_snapshots SET git_operations = ARRAY['receive'] WHERE snapshot_id = $1")
        .bind(snapshot_id)
        .execute(&pool)
        .await
        .expect("remove fetch authority");
    let denied_fetch = repository
        .runtime_git_request(runtime_types::RunId::from_uuid(run_id))
        .await
        .expect("resolve no-fetch request")
        .expect("request remains immutable");
    assert!(denied_fetch.validate().is_err());

    sqlx::query("UPDATE run_git_authority_snapshots SET git_operations = ARRAY['fetch'] WHERE snapshot_id = $1")
        .bind(snapshot_id)
        .execute(&pool)
        .await
        .expect("restore fetch authority");
    sqlx::query(
        "UPDATE run_instance_provenance SET target_ref = 'refs/heads/other' WHERE run_id = $1",
    )
    .bind(run_id)
    .execute(&pool)
    .await
    .expect("move target ref");
    let denied_ref = repository
        .runtime_git_request(runtime_types::RunId::from_uuid(run_id))
        .await
        .expect("resolve out-of-scope ref")
        .expect("request remains immutable");
    assert!(denied_ref.validate().is_err());

    sqlx::query("DELETE FROM run_instance_provenance WHERE run_id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .expect("remove target provenance");
    let missing_target = repository
        .runtime_git_request(runtime_types::RunId::from_uuid(run_id))
        .await
        .expect("missing target denial")
        .expect("immutable capability is retained for fail-closed validation");
    assert!(missing_target.validate().is_err());

    let workspace_id = Uuid::new_v4();
    let metadata = WorkspaceMetadata {
        id: workspace_id,
        state: String::from("preparing"),
        active_path: format!("/tmp/{run_id}"),
        sealed_path: format!("/tmp/{run_id}.sealed"),
        input_commit: Some(target_commit),
    };
    repository
        .insert_runtime_git_preparing(
            &metadata,
            capability_repository,
            metadata.input_commit.as_deref().expect("input commit"),
            runtime_types::RunId::from_uuid(run_id),
            serde_json::json!({"workspace_id": workspace_id}),
        )
        .await
        .expect("atomic runtime workspace row and classification");
    sqlx::query("UPDATE run_workspaces SET state = 'materialization_failed' WHERE run_id = $1")
        .bind(run_id)
        .execute(&pool)
        .await
        .expect("mark runtime materialization failed");
    assert_eq!(
        repository
            .runtime_git_workspace(runtime_types::RunId::from_uuid(run_id))
            .await
            .expect("read failed runtime workspace")
            .expect("failed runtime workspace remains classified")
            .id,
        workspace_id
    );
    let rows = repository
        .runtime_git_workspaces()
        .await
        .expect("runtime workspace recovery rows");
    assert!(rows.iter().any(|(candidate, row)| {
        *candidate == runtime_types::RunId::from_uuid(run_id) && row.id == workspace_id
    }));

    let ordinary_run_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs
         (id, command_id, state, created_at, updated_at, instance_id,
          instance_revision_id, release_id, release_agent_id, attachment_id,
          run_kind, requires_state)
         VALUES ($1, $2, 'provisioning', now(), now(), $3, $4, $5, $6, $7,
                 'normal', false)",
    )
    .bind(ordinary_run_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("seed ordinary run");
    sqlx::query(
        "INSERT INTO run_workspaces
         (id, run_id, repository_id, input_commit, active_path, sealed_path, state)
         VALUES ($1, $2, $3, $4, $5, $6, 'active')",
    )
    .bind(Uuid::new_v4())
    .bind(ordinary_run_id)
    .bind(trigger_repository)
    .bind("b".repeat(40))
    .bind(format!("/tmp/{ordinary_run_id}"))
    .bind(format!("/tmp/{ordinary_run_id}.sealed"))
    .execute(&pool)
    .await
    .expect("seed ordinary workspace");
    assert!(
        repository
            .runtime_git_workspace(runtime_types::RunId::from_uuid(ordinary_run_id))
            .await
            .expect("read ordinary workspace classification")
            .is_none(),
        "ordinary proposal workspaces must never be selected for runtime-Git cleanup"
    );
}
