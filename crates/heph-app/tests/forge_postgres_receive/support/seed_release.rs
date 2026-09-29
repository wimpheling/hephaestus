use super::*;

pub async fn seed_reusable_release(
    pool: &PgPool,
    repository: &Repository,
    build_id: Uuid,
    family_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id)
    .bind(repository.id.as_uuid())
    .bind("d".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed reusable build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'attached')",
    )
    .bind(family_id)
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("seed family");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                 '{}', $6, $7, 'published', now())",
    )
    .bind(release_id)
    .bind(repository.id.as_uuid())
    .bind("d".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'attached', 'Attached', '{}', $4,
                 '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind([4_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release agent");
}

// Explicit fixture IDs keep every seeded foreign-key edge visible in the test.
#[allow(clippy::too_many_arguments)]
pub async fn seed_attached_instance(
    pool: &PgPool,
    repository: &Repository,
    family_id: Uuid,
    release_agent_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    attachment_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(instance_id)
    .bind(repository.project_id.as_uuid())
    .bind(family_id)
    .bind(format!("attached-{}", instance_id.simple()))
    .execute(pool)
    .await
    .expect("seed instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction,
          effective_runtime_policy, effective_policy_hash,
          platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5,
                 'platform/v1', true)",
    )
    .bind(revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate revision");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push')",
    )
    .bind(attachment_id)
    .bind(instance_id)
    .bind(repository.project_id.as_uuid())
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("seed attachment");
}

// This fixture uses a superuser-only replica-mode insert for the immutable
// typed Git snapshot because forge-postgres does not own release capability
// publication. Production rows are created by the runtime authority adapter;
// the receive test only needs the persisted join graph to exercise its
// security-definer lookup against a real PostgreSQL body.
#[allow(clippy::too_many_arguments)]
pub async fn seed_runtime_receive_authority_rows(
    pool: &PgPool,
    run_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Uuid,
    repository_id: Uuid,
    commit: &str,
    snapshot_id: Uuid,
) {
    let mut transaction = pool.begin().await.expect("begin authority fixture");
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *transaction)
        .await
        .expect("enable fixture replica mode");
    sqlx::query(
        "INSERT INTO run_git_authority_snapshots
         (snapshot_id, instance_revision_id, binding_id, repository_id,
          grammar_version, git_operations, ref_globs, changed_path_globs,
          branch_update_policy, branch_create, branch_delete, tag_create,
          tag_update, tag_delete, other_create, other_update, other_delete,
          request_bytes, pack_bytes, object_count, ref_updates,
          exact_parent_required, expected_parent, normalized_hash)
         VALUES ($1, $2, $3, $4, 1, ARRAY['receive'],
                 ARRAY['refs/heads/*'], ARRAY['**'], 'fast_forward_only',
                 false, false, false, false, false, false, false, false,
                 1, 1, 1, 1, false, NULL, $5)",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(Uuid::new_v4())
    .bind(repository_id)
    .bind([9_u8; 32].as_slice())
    .execute(&mut *transaction)
    .await
    .expect("typed Git authority snapshot");
    sqlx::query(
        "INSERT INTO run_instance_provenance
         (run_id, instance_id, instance_revision_id, release_id,
          release_agent_id, attachment_id, target_repository_id, target_ref,
          target_commit, parameter_hash, platform_policy_version, phase,
          authorization_model_version)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'refs/heads/main', $8, $9,
                 'platform/v1', 'normal', 'test/v1')",
    )
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(attachment_id)
    .bind(repository_id)
    .bind(commit)
    .bind([8_u8; 32].as_slice())
    .execute(&mut *transaction)
    .await
    .expect("run provenance");
    transaction
        .commit()
        .await
        .expect("commit authority fixture");
}
