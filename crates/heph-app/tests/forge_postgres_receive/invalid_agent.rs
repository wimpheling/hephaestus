use super::*;

#[tokio::test]
#[serial]
async fn invalid_ui_capture_is_retained_without_agent_config() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let invalid_ui = "version = 1\nunknown_field = \"invalid\"\n";
    let (commit, update) =
        commit_and_update_files(&temporary, &repository, None, Some(invalid_ui)).await;
    let receive = service
        .accept_receive(&repository, ReceiveId::new(), "integration-user", &[update])
        .await
        .expect("invalid UI without agent remains accepted");
    assert!(receive.build_requests.is_empty());
    assert_eq!(receive.run_requests.len(), 1);
    let status: String = sqlx::query_scalar(
        "SELECT status FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("retained invalid UI revision");
    assert_eq!(status, "invalid");
    cleanup(&pool, repository).await;
}
