use super::*;

#[tokio::test]
#[serial]
async fn accepted_ui_capture_links_build_and_replays_without_git() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let (commit, update) =
        commit_and_update_with_ui(&temporary, &repository, &config, STATIC_UI).await;
    let receive_id = ReceiveId::new();
    let first = service
        .accept_receive(
            &repository,
            receive_id,
            "integration-user",
            std::slice::from_ref(&update),
        )
        .await
        .expect("accepted UI receive");
    assert_eq!(first.build_requests.len(), 1);

    let (revision_id, stored_receive, ui_hash): (Uuid, Uuid, Vec<u8>) = sqlx::query_as(
        "SELECT id, receive_id, normalized_ui_hash
         FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("stored valid UI revision");
    assert_eq!(stored_receive, receive_id.as_uuid());
    assert_eq!(ui_hash.len(), 32);

    let link: (Uuid, Uuid, String) = sqlx::query_as(
        "SELECT build_request_id, source_manifest_revision_id, source_commit
         FROM build_request_ui_source_manifests
         WHERE build_request_id = $1",
    )
    .bind(first.build_requests[0].as_uuid())
    .fetch_one(&pool)
    .await
    .expect("build UI link");
    assert_eq!(link.0, first.build_requests[0].as_uuid());
    assert_eq!(link.1, revision_id);
    assert_eq!(link.2, commit.as_str());

    let parsed = agent_config::parse(config.as_bytes());
    let agent = parsed.config.expect("valid agent config");
    let base_hash = agent_config::build_identity::base_build_definition_hash(
        agent.build.as_ref().expect("build config"),
    )
    .expect("base build identity");
    let ui_hash: [u8; 32] = ui_hash.try_into().expect("UI hash width");
    let expected_hash =
        agent_config::build_identity::ui_build_definition_hash(base_hash, ui_hash, None);
    let stored_build_hash: Vec<u8> =
        sqlx::query_scalar("SELECT build_definition_hash FROM build_requests WHERE id = $1")
            .bind(first.build_requests[0].as_uuid())
            .fetch_one(&pool)
            .await
            .expect("UI build identity");
    assert_eq!(stored_build_hash, expected_hash.to_vec());

    tokio::fs::remove_dir_all(temporary.path().join("repositories"))
        .await
        .expect("remove Git storage for replay");
    let replay = service
        .accept_receive(
            &repository,
            receive_id,
            "integration-user",
            std::slice::from_ref(&update),
        )
        .await
        .expect("replay without Git storage");
    assert_eq!(replay.build_requests, first.build_requests);
    assert_eq!(replay.run_requests, first.run_requests);

    cleanup(&pool, repository).await;
}
