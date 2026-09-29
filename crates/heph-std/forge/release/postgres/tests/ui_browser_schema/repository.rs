use super::*;

#[tokio::test]
#[serial]
async fn ui_browser_repository_git_authority_is_explicit_and_live() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping UI browser Git authority: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect bootstrap PostgreSQL role");
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply UI Git migrations");
    let worker = role_pool(&database_url, "hephaestus_worker").await;
    let app = role_pool(&database_url, "hephaestus_app").await;
    let fixture = seed_fixture_reusing_installation_helpers(&worker).await;
    let repository_id: Uuid =
        sqlx::query_scalar("SELECT repository_id FROM ui_installations WHERE id = $1")
            .bind(fixture.repository_installation)
            .fetch_one(&worker)
            .await
            .expect("repository installation target");
    let secret = scoped_secret(fixture.actor, test_secret(fixture.actor, 97));
    insert_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.repository_installation,
        fixture.repository_generation,
        "schema-repository",
        test_secret(fixture.actor, 97),
    )
    .await;
    let digest = UiBrowserSessionSecret::from_bytes(secret)
        .digest()
        .as_bytes()
        .to_vec();
    let allowed: (Uuid, Uuid, String) = sqlx::query_as(
        "SELECT actor_id, repository_id, access
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_one(&app)
    .await
    .expect("approved repository Git read");
    assert_eq!(allowed.0, fixture.actor);
    assert_eq!(allowed.1, repository_id);
    assert_eq!(allowed.2, "read");

    insert_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.no_git_repository_installation,
        fixture.no_git_repository_generation,
        "schema-repository-no-git",
        test_secret(fixture.actor, 99),
    )
    .await;
    let no_git_digest = UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 99))
        .digest()
        .as_bytes()
        .to_vec();
    let no_opt_in = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&no_git_digest)
    .bind(fixture.no_git_repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("no-opt-in denial query");
    assert!(no_opt_in.is_none());

    let wrong_repository = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(fixture.source_project)
    .fetch_optional(&app)
    .await
    .expect("wrong repository denial query");
    assert!(wrong_repository.is_none());

    insert_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.write_repository_installation,
        fixture.write_repository_generation,
        "schema-repository-write",
        test_secret(fixture.actor, 100),
    )
    .await;
    let write_digest = UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 100))
        .digest()
        .as_bytes()
        .to_vec();
    let write_allowed = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'write')",
    )
    .bind(&write_digest)
    .bind(fixture.write_repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("approved repository Git write query");
    assert_eq!(write_allowed, Some(repository_id));

    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke write-level organization grant");
    sqlx::query(
        "DELETE FROM project_maintainers
         WHERE user_id = $1 AND project_id IN ($2, $3)",
    )
    .bind(fixture.actor)
    .bind(fixture.project)
    .bind(fixture.source_project)
    .execute(&worker)
    .await
    .expect("revoke direct project write grants");
    sqlx::query("DELETE FROM repository_managers WHERE repository_id = $1 AND user_id = $2")
        .bind(repository_id)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke direct repository write grant");
    let write_without_grant = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'write')",
    )
    .bind(&write_digest)
    .bind(fixture.write_repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("revoked write grant denial query");
    assert!(write_without_grant.is_none());
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner') ON CONFLICT (organization_id, user_id)
         DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(fixture.organization)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("restore write-level organization grant");
    let write_without_declaration = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'write')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("read-only write denial query");
    assert!(write_without_declaration.is_none());

    let stale_generation = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.other_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("stale generation denial query");
    assert!(stale_generation.is_none());

    let unknown_child = UiBrowserSessionSecret::from_bytes(test_secret(fixture.actor, 98))
        .digest()
        .as_bytes()
        .to_vec();
    let revoked_child = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&unknown_child)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("unknown child denial query");
    assert!(revoked_child.is_none());

    let expired_secret = test_secret(fixture.actor, 101);
    insert_expired_authenticated_child_for_installation(
        &worker,
        &fixture,
        fixture.repository_installation,
        fixture.repository_generation,
        "schema-repository",
        expired_secret,
    )
    .await;
    let expired_digest = UiBrowserSessionSecret::from_bytes(expired_secret)
        .digest()
        .as_bytes()
        .to_vec();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let expired_child = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&expired_digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("expired child denial query");
    assert!(expired_child.is_none());

    sqlx::query("UPDATE ui_installations SET lifecycle = 'disabled' WHERE id = $1")
        .bind(fixture.repository_installation)
        .execute(&worker)
        .await
        .expect("disable repository UI installation");
    let disabled_installation = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("disabled installation denial query");
    assert!(disabled_installation.is_none());
    sqlx::query("UPDATE ui_installations SET lifecycle = 'enabled' WHERE id = $1")
        .bind(fixture.repository_installation)
        .execute(&worker)
        .await
        .expect("restore repository UI installation");

    sqlx::query("DELETE FROM organization_members WHERE organization_id = $1 AND user_id = $2")
        .bind(fixture.organization)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("revoke live repository Git grants");
    let revoked_grants = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("revoked grants denial query");
    assert!(revoked_grants.is_none());
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner') ON CONFLICT (organization_id, user_id)
         DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(fixture.organization)
    .bind(fixture.actor)
    .execute(&worker)
    .await
    .expect("restore live repository Git grants");

    // Release revocation is intentionally terminal, so keep this mutation as
    // the final assertion in the disposable fixture.
    sqlx::query(
        "UPDATE releases
         SET state = 'revoked', revoked_at = statement_timestamp()
         WHERE id = $1",
    )
    .bind(fixture.release)
    .execute(&worker)
    .await
    .expect("revoke published release");
    let revoked_release = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("revoked release denial query");
    assert!(revoked_release.is_none());
    sqlx::query(
        "UPDATE human_browser_sessions
         SET revoked_at = statement_timestamp(), revocation_reason = 'logout'
         WHERE id = $1",
    )
    .bind(fixture.parent_session)
    .execute(&worker)
    .await
    .expect("revoke repository parent session");
    let revoked_parent = sqlx::query_scalar::<_, Uuid>(
        "SELECT repository_id
         FROM resolve_ui_browser_repository_git_access($1, $2, $3, 'read')",
    )
    .bind(&digest)
    .bind(fixture.repository_generation)
    .bind(repository_id)
    .fetch_optional(&app)
    .await
    .expect("revoked parent denial query");
    assert!(revoked_parent.is_none());
    println!(
        "REAL_UI_BROWSER_REPOSITORY_GIT_AUTHORITY=1 opt_in=1 no_opt_in=1 wrong_repository=1 read_only_write_denied=1 write_allowed=1 write_grant_revoked=1 stale_generation=1 child_expired=1 parent_revoked=1 installation_disabled=1 release_revoked=1 grants_revoked=1"
    );
}
