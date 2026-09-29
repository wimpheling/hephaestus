use super::*;

#[tokio::test]
#[serial]
async fn invalid_ui_capture_accepts_ref_and_run_but_skips_build() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let invalid_ui = "version = 1\nunknown_field = \"invalid\"\n";
    let (commit, update) =
        commit_and_update_with_ui(&temporary, &repository, &config, invalid_ui).await;
    let receive = service
        .accept_receive(&repository, ReceiveId::new(), "integration-user", &[update])
        .await
        .expect("invalid UI receive remains accepted");
    assert!(receive.build_requests.is_empty());
    assert_eq!(receive.run_requests.len(), 1);
    let (status, diagnostics): (String, serde_json::Value) = sqlx::query_as(
        "SELECT status, diagnostics FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("invalid UI revision");
    assert_eq!(status, "invalid");
    assert!(!diagnostics.as_array().expect("diagnostic array").is_empty());
    let linked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM build_request_ui_source_manifests
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("invalid UI build links");
    assert_eq!(linked, 0);
    cleanup(&pool, repository).await;
}
