use super::*;

pub(super) async fn verify(admin_pool: &PgPool, service: &ReleaseService) -> String {
    let success_fixture = seed(admin_pool).await;
    let success_gateway = managed_gateway_manifest("reviewer");
    attach_ui_capture(
        admin_pool,
        success_fixture.build,
        managed_api_manifest(),
        Some(success_gateway.as_bytes()),
        None,
    )
    .await;
    let success_release = ReleaseId::new();
    let success_agent = ReleaseAgentId::new();
    let success_artifact = release_test_artifact(
        "bin/reviewer",
        ArtifactKind::Executable,
        "application/octet-stream",
        20,
    );
    let success_key = key("complete-managed-api", success_release.as_uuid());
    service
        .complete_build(CompleteBuild {
            command_key: success_key,
            build_request_id: success_fixture.build,
            release_id: success_release,
            version: ReleaseVersion::parse("managed-api-v1").expect("release version"),
            release_agent_id: success_agent,
            artifacts: vec![success_artifact.clone()],
        })
        .await
        .expect("complete managed/API release");
    let stored_bindings: (Uuid, String, Uuid, String, String) = sqlx::query_as(
        "SELECT managed.release_agent_id, managed.route,
            api.release_agent_id, api.method, api.route
     FROM release_ui_managed_services AS managed
     JOIN release_ui_api_bindings AS api
       ON api.release_id = managed.release_id
      AND api.ui_key = managed.ui_key
     WHERE managed.release_id = $1 AND managed.ui_key = 'assistant'
       AND api.api_key = 'service-api'",
    )
    .bind(success_release.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("managed/API bindings");
    assert_eq!(
        stored_bindings,
        (
            success_agent.as_uuid(),
            String::from("/service/ui"),
            success_agent.as_uuid(),
            String::from("GET"),
            String::from("/service/api"),
        )
    );
    let replay = service
        .complete_build(CompleteBuild {
            command_key: success_key,
            build_request_id: success_fixture.build,
            release_id: ReleaseId::new(),
            version: ReleaseVersion::parse("ignored-replay").expect("release version"),
            release_agent_id: ReleaseAgentId::new(),
            artifacts: vec![release_test_artifact(
                "ignored-replay",
                ArtifactKind::File,
                "text/plain",
                1,
            )],
        })
        .await
        .expect("managed/API replay");
    assert_eq!(replay, success_release);
    let replay_rows: (i64, i64, i64) = sqlx::query_as(
        "SELECT
         (SELECT count(*) FROM release_ui_source_snapshots WHERE release_id = $1),
         (SELECT count(*) FROM release_ui_managed_services WHERE release_id = $1),
         (SELECT count(*) FROM release_ui_api_bindings WHERE release_id = $1)",
    )
    .bind(success_release.as_uuid())
    .fetch_one(admin_pool)
    .await
    .expect("replay row counts");
    assert_eq!(replay_rows, (1, 1, 1));
    success_gateway
}
