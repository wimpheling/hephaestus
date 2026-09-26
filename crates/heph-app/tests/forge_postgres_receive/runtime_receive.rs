use super::*;

#[tokio::test]
#[serial]
async fn runtime_receive_persists_provenance_suppresses_origin_and_replays() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let (origin_attachment, instance_id, project_id): (Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT id, instance_id, project_id
         FROM agent_attachments
         WHERE repository_id = $1
         ORDER BY id
         LIMIT 1",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("origin attachment");
    let sibling_attachment = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector, trigger_policy)
         VALUES ($1, $2, $3, $4, 'refs/heads/*', 'push')",
    )
    .bind(sibling_attachment)
    .bind(instance_id)
    .bind(project_id)
    .bind(repository.id.as_uuid())
    .execute(&pool)
    .await
    .expect("sibling attachment");

    let (revision_id, release_id, release_agent_id): (Uuid, Uuid, Uuid) = sqlx::query_as(
        "SELECT revision.id, release.id, release_agent.id
         FROM agent_instance_revisions AS revision
         JOIN release_agents AS release_agent ON release_agent.id = revision.release_agent_id
         JOIN releases AS release ON release.id = release_agent.release_id
         WHERE revision.instance_id = $1",
    )
    .bind(instance_id)
    .fetch_one(&pool)
    .await
    .expect("origin revision");
    let run_id = Uuid::new_v4();
    let command_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO runs
         (id, command_id, state, created_at, updated_at,
          instance_id, instance_revision_id, release_id, release_agent_id,
          attachment_id, run_kind, requires_state)
         VALUES ($1, $2, 'queued', now(), now(), $3, $4, $5, $6, $7,
                 'normal', false)",
    )
    .bind(run_id)
    .bind(command_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind(origin_attachment)
    .execute(&pool)
    .await
    .expect("runtime run");
    let snapshot_id = Uuid::new_v4();
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
    .expect("runtime snapshot");
    let runtime_session_id = Uuid::new_v4();
    let runtime_credential_hash =
        [Uuid::new_v4().into_bytes(), Uuid::new_v4().into_bytes()].concat();
    sqlx::query(
        "INSERT INTO runtime_authority_sessions
         (id, snapshot_id, run_id, instance_id, instance_revision_id,
          attachment_id, identity_hash, snapshot_hash, issuance_generation,
          credential_hash, status, issued_at, expires_at, acknowledged_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, $9, 'active',
                 now(), now() + interval '10 minutes', now())",
    )
    .bind(runtime_session_id)
    .bind(snapshot_id)
    .bind(run_id)
    .bind(instance_id)
    .bind(revision_id)
    .bind(origin_attachment)
    .bind([2_u8; 32].as_slice())
    .bind([1_u8; 32].as_slice())
    .bind(runtime_credential_hash.as_slice())
    .execute(&pool)
    .await
    .expect("runtime session");

    let (_, update) = commit_and_update(
        &temporary,
        &repository,
        &valid_config(repository.id.as_uuid()),
    )
    .await;
    seed_runtime_receive_authority_rows(
        &pool,
        run_id,
        instance_id,
        revision_id,
        release_id,
        release_agent_id,
        origin_attachment,
        repository.id.as_uuid(),
        update.new_commit.as_ref().expect("runtime commit").as_str(),
        snapshot_id,
    )
    .await;
    let receive_id = ReceiveId::new();
    let first = service
        .accept_runtime_receive(
            &repository,
            receive_id,
            RuntimeReceiveProvenance { runtime_session_id },
            std::slice::from_ref(&update),
        )
        .await
        .expect("runtime receive");
    assert!(first.run_requests.is_empty());

    let stored: (Option<Uuid>, Option<Uuid>) = sqlx::query_as(
        "SELECT runtime_session_id, runtime_attachment_id
         FROM git_receives WHERE id = $1",
    )
    .bind(receive_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("runtime receive provenance");
    assert_eq!(stored, (Some(runtime_session_id), Some(origin_attachment)));

    let wrong_repository = forge_domain::RepositoryId::new();
    let wrong_repository_resolution: Option<(Option<Uuid>,)> =
        sqlx::query_as("SELECT * FROM resolve_runtime_receive_attachment($1, $2)")
            .bind(runtime_session_id)
            .bind(wrong_repository.as_uuid())
            .fetch_optional(&pool)
            .await
            .expect("wrong repository resolution");
    assert!(wrong_repository_resolution.is_none());

    let detached_session = seed_detached_runtime_session(
        &pool,
        &repository,
        instance_id,
        revision_id,
        release_id,
        release_agent_id,
    )
    .await;
    let detached = service
        .accept_runtime_receive(
            &repository,
            ReceiveId::new(),
            RuntimeReceiveProvenance {
                runtime_session_id: detached_session,
            },
            std::slice::from_ref(&update),
        )
        .await
        .expect("runtime receive without originating attachment");
    assert!(detached.run_requests.is_empty());
    let detached_provenance: Option<Uuid> = sqlx::query_scalar(
        "SELECT runtime_attachment_id FROM git_receives
         WHERE runtime_session_id = $1",
    )
    .bind(detached_session)
    .fetch_one(&pool)
    .await
    .expect("detached runtime provenance");
    assert_eq!(detached_provenance, None);

    let duplicate_session = service
        .accept_runtime_receive(
            &repository,
            receive_id,
            RuntimeReceiveProvenance {
                runtime_session_id: detached_session,
            },
            std::slice::from_ref(&update),
        )
        .await;
    assert!(matches!(
        duplicate_session,
        Err(forge_service::ForgeRepositoryError::ReceiveConflict(_))
    ));

    let replay = service
        .accept_runtime_receive(
            &repository,
            receive_id,
            RuntimeReceiveProvenance { runtime_session_id },
            std::slice::from_ref(&update),
        )
        .await
        .expect("runtime receive replay");
    assert_eq!(replay.run_requests, first.run_requests);

    cleanup(&pool, repository).await;
}
