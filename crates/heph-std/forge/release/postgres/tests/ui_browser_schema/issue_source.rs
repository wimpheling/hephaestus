use super::*;
use crate::issue::IssueContext;

#[allow(clippy::too_many_lines)]
pub async fn run(ctx: &IssueContext) {
    let worker = ctx.worker.clone();
    let store = &ctx.store;
    let fixture = ctx.fixture;
    let actor = ctx.actor;
    let parent = ctx.parent;
    let installation = ctx.installation;
    let route = ctx.route.clone();
    let baseline = ctx.baseline;
    let stale_generation = ctx.stale_generation;

    sqlx::query("UPDATE organization_members SET role = 'member' WHERE organization_id = $1 AND user_id = $2")
    .bind(fixture.organization)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("demote actor for source permission case");
    sqlx::query(
        "INSERT INTO project_maintainers (project_id, user_id)
     SELECT project_id, $2 FROM ui_installations WHERE id = $1
     ON CONFLICT DO NOTHING",
    )
    .bind(fixture.other_installation)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("grant source project permission");
    let source_permission_handoff = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.other_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            route: UiBrowserRoute::parse("schema-ui-two").expect("source permission route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await
        .expect("project maintainer may use source release");
    let with_source_permission: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
            .fetch_one(&worker)
            .await
            .expect("count source permission handoff");
    assert_eq!(with_source_permission, baseline + 1);
    assert_eq!(source_permission_handoff.route.as_str(), "schema-ui-two");
    sqlx::query(
        "DELETE FROM project_maintainers
     WHERE project_id = (SELECT project_id FROM ui_installations WHERE id = $1)
       AND user_id = $2",
    )
    .bind(fixture.other_installation)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("revoke source project permission");
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke source release permission");
    let source_permission_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.other_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            route: UiBrowserRoute::parse("schema-ui-two").expect("source permission route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(
        source_permission_denied,
        Err(UiBrowserHandoffError::PermissionDenied)
    );
    sqlx::query("UPDATE organization_members SET role = 'owner' WHERE organization_id = $1 AND user_id = $2")
    .bind(fixture.organization)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("restore actor owner role");

    let source_release: Uuid =
        sqlx::query_scalar("SELECT release_id FROM ui_installation_generations WHERE id = $1")
            .bind(stale_generation)
            .fetch_one(&worker)
            .await
            .expect("read source release");
    sqlx::query(
        "UPDATE releases
     SET state = 'revoked', revoked_at = statement_timestamp()
     WHERE id = $1",
    )
    .bind(source_release)
    .execute(&worker)
    .await
    .expect("revoke source release");
    let source_revoked = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: installation,
            generation_id: UiInstallationGenerationId::from_uuid(stale_generation),
            route,
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(source_revoked, Err(UiBrowserHandoffError::PermissionDenied));

    sqlx::query("UPDATE ui_installations SET lifecycle = 'removed', removed_at = statement_timestamp() WHERE id = $1")
    .bind(fixture.other_installation)
    .execute(&worker)
    .await
    .expect("remove alternate installation");
    let removed_denied = store
        .create_ui_browser_handoff(CreateUiBrowserHandoff {
            request_id: RequestId::new(),
            actor_id: actor,
            parent_session_id: parent,
            installation_id: UiInstallationId::from_uuid(fixture.other_installation),
            generation_id: UiInstallationGenerationId::from_uuid(fixture.other_generation),
            route: UiBrowserRoute::parse("schema-ui-two").expect("alternate route"),
            secret: UiBrowserHandoffSecret::random(),
        })
        .await;
    assert_eq!(removed_denied, Err(UiBrowserHandoffError::PermissionDenied));

    let final_count: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_browser_handoffs")
        .fetch_one(&worker)
        .await
        .expect("count final denied attempts");
    assert_eq!(final_count, baseline + 1);
}
