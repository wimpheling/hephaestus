use super::*;

pub(super) async fn verify(context: &MatrixContext<'_>, cases: &mut CaseCounter) {
    let admin_pool = context.admin_pool;
    let service = context.service;
    let fixture = context.fixture;
    let release_id = context.release_id;
    let source_repository = context.source_repository;
    let source_project = context.source_project;
    let target = seed_same_org_target_project(admin_pool, fixture, "source-read").await;
    // Keep release CanUse available through the repository's public-read
    // relation while removing every project-read path below.
    sqlx::query("UPDATE repositories SET is_public = true WHERE id = $1")
        .bind(source_repository)
        .execute(admin_pool)
        .await
        .expect("enable source repository public read");
    // Organization membership grants project read transitively. Remove that
    // tuple as well as the direct source maintainer row so this case tests the
    // source-project barrier rather than an alternate organization path.
    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization.as_uuid())
        .bind(fixture.actor.as_uuid())
        .execute(admin_pool)
        .await
        .expect("revoke source organization read");
    sqlx::query("DELETE FROM project_maintainers WHERE project_id = $1 AND user_id = $2")
        .bind(source_project)
        .bind(fixture.actor.as_uuid())
        .execute(admin_pool)
        .await
        .expect("revoke source project read");
    assert_installation_denied_without_receipt(
        service,
        admin_pool,
        fixture.actor,
        target,
        release_id,
        cases.next("source-read"),
    )
    .await;
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(source_project)
        .bind(fixture.actor.as_uuid())
        .execute(admin_pool)
        .await
        .expect("restore source project read");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member')",
    )
    .bind(fixture.organization.as_uuid())
    .bind(fixture.actor.as_uuid())
    .execute(admin_pool)
    .await
    .expect("restore source organization read");
    sqlx::query("UPDATE repositories SET is_public = false WHERE id = $1")
        .bind(source_repository)
        .execute(admin_pool)
        .await
        .expect("restore source repository visibility");

    let target = seed_same_org_target_project(admin_pool, fixture, "agent-use").await;
    // Release-agent use is derived from the release lifecycle. Revocation is
    // terminal, so exercise the real revoked state and its redacted category
    // as the final case in this disposable fixture.
    sqlx::query("UPDATE releases SET state = 'revoked', revoked_at = now() WHERE id = $1")
        .bind(release_id.as_uuid())
        .execute(admin_pool)
        .await
        .expect("revoke release for agent-use denial");
    assert_installation_denied_without_receipt_as(
        service,
        admin_pool,
        fixture.actor,
        target,
        release_id,
        cases.next("agent-use"),
        UiInstallationError::PermissionDenied,
    )
    .await;
}
