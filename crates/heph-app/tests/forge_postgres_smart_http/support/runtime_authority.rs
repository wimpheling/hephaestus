use super::*;

pub fn runtime_receive_binding_hash(repository_id: Uuid) -> [u8; 32] {
    let ceiling = GitCapabilityCeiling::new(GitCapabilityCeilingInput {
        operations: vec![CapabilityGitOperation::Receive],
        ref_globs: vec![
            RefGlob::parse_explicitly_broad("refs/heads/main").expect("runtime ref glob"),
        ],
        changed_path_globs: vec![
            ChangedPathGlob::parse_explicitly_broad("runtime.txt").expect("runtime path glob"),
        ],
        update_policy: RefUpdatePolicy {
            branches: BranchRefPolicy {
                updates: BranchUpdatePolicy::FastForwardOnly,
                create: RefMutationPermission::Allow,
                delete: RefMutationPermission::Deny,
            },
            tags: RefNamespacePolicy::default(),
            other: RefNamespacePolicy::default(),
        },
        transfer_limits: TransferLimits::new(16_777_216, 1_073_741_824, 1_000_000, 256)
            .expect("runtime transfer limits"),
        exact_parent_required: false,
    })
    .expect("runtime capability ceiling");
    *BoundGitCapability::new(
        CapabilityRepositoryId::new(repository_id),
        ceiling.clone(),
        &ceiling,
    )
    .expect("runtime bound capability")
    .normalized_hash()
    .expect("runtime capability hash")
    .as_bytes()
}

pub fn credential_header(credential: &RuntimeGitCredential) -> String {
    let token = credential.expose_token().to_string();
    format!(
        "Basic {}",
        BASE64_STANDARD.encode(format!("heph-runtime:{token}"))
    )
}

// The transport test uses the production Git HTTP/authentication path while
// constructing only the already-published runtime join graph that this crate
// does not own. Release-authority integration tests cover creation of these
// immutable rows through the runtime authority adapter.
#[allow(clippy::too_many_lines)]
pub async fn seed_runtime_transport_authority(
    pool: &PgPool,
    repository_id: Uuid,
    commit: &str,
    user_id: UserId,
    handoff_root: std::path::PathBuf,
    existing_binding_id: Option<Uuid>,
    lifetime: time::Duration,
) -> (Uuid, Uuid, RuntimeGitCredential, Uuid) {
    let (attachment_id, instance_id): (Uuid, Uuid) =
        sqlx::query_as("SELECT id, instance_id FROM agent_attachments WHERE repository_id = $1")
            .bind(repository_id)
            .fetch_one(pool)
            .await
            .expect("runtime transport attachment");
    let (revision_id, release_id, release_agent_id): (Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT revision.id, release.id, release_agent.id
         FROM agent_instance_revisions AS revision
         JOIN release_agents AS release_agent ON release_agent.id = revision.release_agent_id
         JOIN releases AS release ON release.id = release_agent.release_id
         WHERE revision.instance_id = $1",
    )
    .bind(instance_id)
    .fetch_one(pool)
    .await
    .expect("runtime transport revision");
    let run_id = Uuid::new_v4();
    let snapshot_id = Uuid::new_v4();
    let runtime_session_id = Uuid::new_v4();
    let binding_id = existing_binding_id.unwrap_or_else(Uuid::new_v4);
    let now = time::OffsetDateTime::now_utc();
    let expires_at = now + lifetime;
    let binding_hash = runtime_receive_binding_hash(repository_id);
    sqlx::query(
        "INSERT INTO runs
         (id, command_id, state, created_at, updated_at,
          instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, run_kind, requires_state)
         VALUES ($1, $2, 'queued', now(), now(), $3, $4, $5, $6, $7,
                 'normal', false)",
    )
    .bind(run_id)
    .bind(Uuid::new_v4())
    .bind(instance_id)
    .bind(revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(attachment_id)
    .execute(pool)
    .await
    .expect("runtime transport run");
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
    .execute(pool)
    .await
    .expect("runtime transport snapshot");
    let session_credential_hash =
        [Uuid::new_v4().into_bytes(), Uuid::new_v4().into_bytes()].concat();
    sqlx::query(
        "INSERT INTO runtime_authority_sessions
         (id, snapshot_id, run_id, instance_id, instance_revision_id,
          attachment_id, identity_hash, snapshot_hash, issuance_generation,
          credential_hash, status, issued_at, expires_at, acknowledged_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, $9, 'pending_handoff',
                 $10, $11, NULL)",
    )
    .bind(runtime_session_id)
    .bind(snapshot_id)
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind(attachment_id)
    .bind([2_u8; 32].as_slice())
    .bind([1_u8; 32].as_slice())
    .bind(session_credential_hash.as_slice())
    .bind(now)
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("runtime transport session");
    let mut transaction = pool.begin().await.expect("begin runtime transport graph");
    sqlx::query("SET LOCAL session_replication_role = 'replica'")
        .execute(&mut *transaction)
        .await
        .expect("enable runtime transport fixture mode");
    if existing_binding_id.is_none() {
        sqlx::query(
            "INSERT INTO agent_capability_bindings
             (id, instance_revision_id, release_agent_id, requirement_id,
              requirement_hash, slot_key, resource_kind, resource_id,
              granted_operations, normalized_hash, authorization_model_version,
              created_by)
             VALUES ($1, $2, $3, $4, $5, 'content', 'repository', $6,
                     ARRAY['update_ref'], $7, 'test/v1', $8)",
        )
        .bind(binding_id)
        .bind(revision_id)
        .bind(release_agent_id)
        .bind(Uuid::new_v4())
        .bind([3_u8; 32].as_slice())
        .bind(repository_id)
        .bind(binding_hash.as_slice())
        .bind(user_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .expect("runtime generic capability binding");
    }
    sqlx::query(
        "INSERT INTO run_authorization_snapshot_bindings
         (snapshot_id, instance_revision_id, ordinal, binding_id,
          binding_hash, slot_key, resource_kind, resource_id,
          granted_operations)
         VALUES ($1, $2, 0, $3, $4, 'content', 'repository', $5,
                 ARRAY['update_ref'])",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(binding_id)
    .bind(binding_hash.as_slice())
    .bind(repository_id)
    .execute(&mut *transaction)
    .await
    .expect("runtime snapshot capability binding");
    sqlx::query(
        "INSERT INTO run_git_authority_snapshots
         (snapshot_id, instance_revision_id, binding_id, repository_id,
          grammar_version, git_operations, ref_globs, changed_path_globs,
          branch_update_policy, branch_create, branch_delete, tag_create,
          tag_update, tag_delete, other_create, other_update, other_delete,
          request_bytes, pack_bytes, object_count, ref_updates,
          exact_parent_required, expected_parent, normalized_hash)
         VALUES ($1, $2, $3, $4, 1, ARRAY['receive'],
                 ARRAY['refs/heads/main'], ARRAY['runtime.txt'], 'fast_forward_only',
                 true, false, false, false, false, false, false, false,
                 16777216, 1073741824, 1000000, 256, false, NULL, $5)",
    )
    .bind(snapshot_id)
    .bind(revision_id)
    .bind(binding_id)
    .bind(repository_id)
    .bind(binding_hash.as_slice())
    .execute(&mut *transaction)
    .await
    .expect("runtime transport Git snapshot");
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
    .expect("runtime transport provenance");
    transaction
        .commit()
        .await
        .expect("commit runtime transport graph");
    let handoff = EncryptedFileRuntimeGitHandoffStore::new(handoff_root, [0x11; 32])
        .expect("runtime Git handoff store");
    let credential_issuer = RuntimeGitCredentialIssuer::new(
        PgRuntimeGitCredentialRepository::new(pool.clone()),
        handoff,
    );
    let issued_credential = credential_issuer
        .issue(
            RuntimeSessionId::from_uuid(runtime_session_id),
            RuntimeCredentialGeneration::INITIAL,
            expires_at,
            now,
        )
        .await
        .expect("issue runtime Git credential");
    sqlx::query(
        "UPDATE runtime_authority_sessions
         SET status = 'active', acknowledged_at = $2
         WHERE id = $1",
    )
    .bind(runtime_session_id)
    .bind(now)
    .execute(pool)
    .await
    .expect("activate runtime Git session");
    let permission: i32 = sqlx::query_scalar(
        "SELECT check_permission(
             'agent_instance', $1, 'agent_update_ref', 'repository', $2
         )",
    )
    .bind(instance_id.to_string())
    .bind(repository_id.to_string())
    .fetch_one(pool)
    .await
    .expect("runtime capability permission");
    assert_eq!(permission, 1, "runtime capability permission denied");
    let authenticated: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authenticate_runtime_git_credential($1, $2, 'receive')",
    )
    .bind(
        issued_credential
            .credential
            .storage_hash()
            .as_bytes()
            .as_slice(),
    )
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("runtime credential authentication query");
    assert_eq!(authenticated, 1, "production runtime credential rejected");
    (
        runtime_session_id,
        attachment_id,
        issued_credential.credential,
        binding_id,
    )
}
