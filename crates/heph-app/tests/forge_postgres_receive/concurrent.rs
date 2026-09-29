use super::*;

#[tokio::test]
#[serial]
async fn concurrent_receives_same_repository_serialize_ui_capture() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let config = valid_config(repository.id.as_uuid());
    let (commit, update) =
        commit_and_update_with_ui(&temporary, &repository, &config, STATIC_UI).await;
    let first_service = service.clone();
    let second_service = service.clone();
    let first_repository = repository.clone();
    let second_repository = repository.clone();
    let first_update = update.clone();
    let second_update = update.clone();
    let joined = tokio::time::timeout(Duration::from_secs(30), async move {
        tokio::join!(
            first_service.accept_receive(
                &first_repository,
                ReceiveId::new(),
                "concurrent-first",
                std::slice::from_ref(&first_update),
            ),
            second_service.accept_receive(
                &second_repository,
                ReceiveId::new(),
                "concurrent-second",
                std::slice::from_ref(&second_update),
            ),
        )
    })
    .await
    .expect("concurrent receives must not deadlock");
    let first = joined.0.expect("first concurrent receive");
    let second = joined.1.expect("second concurrent receive");
    assert_eq!(first.build_requests, second.build_requests);

    let captures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ui_source_manifest_revisions
         WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository.id.as_uuid())
    .bind(commit.as_str())
    .fetch_one(&pool)
    .await
    .expect("capture count");
    assert_eq!(captures, 1, "same commit has one immutable capture");
    cleanup(&pool, repository).await;
}
