use super::*;

#[tokio::test]
#[serial]
async fn same_commit_on_multiple_refs_reuses_capture_and_links_each_build() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid()).replace(
        "triggers = [\"refs/heads/main\"]",
        "triggers = [\"refs/heads/main\", \"refs/heads/feature\"]",
    );
    let (commit, main_update) =
        commit_and_update_with_ui(&temporary, &repository, &config, STATIC_UI).await;
    let feature_update = RefUpdate {
        git_ref: GitRef::parse("refs/heads/feature").expect("feature ref"),
        old_commit: None,
        new_commit: Some(commit.clone()),
    };
    let receive = service
        .accept_receive(
            &repository,
            ReceiveId::new(),
            "integration-user",
            &[main_update, feature_update],
        )
        .await
        .expect("accepted multi-ref receive");
    assert_eq!(receive.build_requests.len(), 2);
    let revisions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("single shared UI revision");
    assert_eq!(revisions, 1);
    let links: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM build_request_ui_source_manifests
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("two UI build links");
    assert_eq!(links, 2);
    cleanup(&pool, repository).await;
}
