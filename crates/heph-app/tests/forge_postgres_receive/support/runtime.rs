use super::*;

pub async fn seed_detached_runtime_session(
    pool: &PgPool,
    repository: &Repository,
    instance_id: Uuid,
    revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
) -> Uuid {
    let run_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let now = time::OffsetDateTime::now_utc();
    let expires_at = now + time::Duration::minutes(10);
    sqlx::query(
        "INSERT INTO runs
         (id, command_id, state, created_at, updated_at,
          instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, run_kind, requires_state)
         VALUES ($1, $2, 'queued', $3, $3, $4, $5, $6, $7, NULL, 'update', false)",
    )
    .bind(run_id)
    .bind(Uuid::new_v4())
    .bind(now)
    .bind(instance_id)
    .bind(revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .execute(pool)
    .await
    .expect("detached runtime run");
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
    .bind([14_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("detached runtime snapshot");
    let credential_hash = [Uuid::new_v4().into_bytes(), Uuid::new_v4().into_bytes()].concat();
    sqlx::query(
        "INSERT INTO runtime_authority_sessions
         (id, snapshot_id, run_id, instance_id, instance_revision_id,
          attachment_id, identity_hash, snapshot_hash, issuance_generation,
          credential_hash, status, issued_at, expires_at, acknowledged_at)
         VALUES ($1, $2, $3, $4, $5, NULL, $6, $7, 1, $8, 'active', $9, $10, $9)",
    )
    .bind(session_id)
    .bind(snapshot_id)
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind([15_u8; 32].as_slice())
    .bind([14_u8; 32].as_slice())
    .bind(credential_hash.as_slice())
    .bind(now)
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("detached runtime session");
    let mut transaction = pool.begin().await.expect("begin detached runtime graph");
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *transaction)
        .await
        .expect("enable detached fixture mode");
    sqlx::query(
        "INSERT INTO run_git_authority_snapshots
         (snapshot_id, instance_revision_id, binding_id, repository_id,
          grammar_version, git_operations, ref_globs, changed_path_globs,
          branch_update_policy, branch_create, branch_delete, tag_create,
          tag_update, tag_delete, other_create, other_update, other_delete,
          request_bytes, pack_bytes, object_count, ref_updates,
          exact_parent_required, expected_parent, normalized_hash)
         VALUES ($1, $2, $3, $4, 1, ARRAY['receive'],
                 ARRAY['refs/heads/main'], ARRAY['**'], 'fast_forward_only',
                 false, false, false, false, false, false, false, false,
                 1, 1, 1, 1, false, NULL, $5)",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(Uuid::new_v4())
    .bind(repository.id.as_uuid())
    .bind([16_u8; 32].as_slice())
    .execute(&mut *transaction)
    .await
    .expect("detached runtime Git snapshot");
    transaction
        .commit()
        .await
        .expect("commit detached runtime graph");
    session_id
}
