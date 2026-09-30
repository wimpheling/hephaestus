use serial_test::serial;
use std::env;
use uuid::Uuid;

use super::support::{
    admin_pool, assert_sqlstate, role_pool, seed_global_installation, seed_parent_rows,
    seed_second_published_fixture,
};

#[tokio::test]
#[serial]
// Keep this end-to-end RLS proof together so tenant scope and dual-membership
// behavior are checked against one seeded schema state.
#[allow(clippy::too_many_lines)]
async fn global_ui_installation_scope_tenant_invariant_and_dual_membership_rls() {
    let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        eprintln!("skipping global UI installation schema: test URL is unset");
        return;
    };
    let bootstrap = admin_pool(&database_url).await;
    sqlx::migrate!("../../../../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations through 0088");
    let max_migration: i64 = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(version) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&bootstrap)
    .await
    .expect("read migration marker")
    .expect("migration marker exists");
    assert!(max_migration >= 88);

    let worker = role_pool(&database_url, "hephaestus_worker", Uuid::nil()).await;
    let member_a = Uuid::new_v4();
    let member_b = Uuid::new_v4();
    let outsider = Uuid::new_v4();
    let organization_b = Uuid::new_v4();
    let project_b = Uuid::new_v4();
    let repository_b = Uuid::new_v4();
    let unreferenced_project = Uuid::new_v4();
    let unreferenced_repository = Uuid::new_v4();
    let source_fixture = seed_parent_rows(&worker).await;
    let actor = source_fixture.actor;
    let organization_a: Uuid =
        sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1")
            .bind(source_fixture.project)
            .fetch_one(&worker)
            .await
            .expect("read source organization");
    sqlx::query(
        "INSERT INTO users (id, display_name) VALUES
         ($1, 'global member A'), ($2, 'global member B'),
         ($3, 'global outsider')",
    )
    .bind(member_a)
    .bind(member_b)
    .bind(outsider)
    .execute(&worker)
    .await
    .expect("seed global users");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member')",
    )
    .bind(organization_a)
    .bind(member_a)
    .execute(&worker)
    .await
    .expect("seed organization A member");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'Global UI org B')")
        .bind(organization_b)
        .execute(&worker)
        .await
        .expect("seed organization B");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'member'), ($1, $3, 'member')",
    )
    .bind(organization_b)
    .bind(actor)
    .bind(member_b)
    .execute(&worker)
    .await
    .expect("seed organization B members");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'global-b')")
        .bind(project_b)
        .bind(organization_b)
        .execute(&worker)
        .await
        .expect("seed organization B project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, 'global-b-repository')",
    )
    .bind(repository_b)
    .bind(project_b)
    .execute(&worker)
    .await
    .expect("seed organization B repository");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'unreferenced')")
        .bind(unreferenced_project)
        .bind(organization_a)
        .execute(&worker)
        .await
        .expect("seed unreferenced project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name)
         VALUES ($1, $2, 'unreferenced-repository')",
    )
    .bind(unreferenced_repository)
    .bind(unreferenced_project)
    .execute(&worker)
    .await
    .expect("seed unreferenced repository");
    let (release_b, _, _, _) =
        seed_second_published_fixture(&worker, actor, project_b, repository_b).await;

    let installation_a = Uuid::new_v4();
    let generation_a = Uuid::new_v4();
    seed_global_installation(
        &worker,
        organization_a,
        installation_a,
        generation_a,
        source_fixture.second_release,
        "schema-global-two",
    )
    .await;
    let installation_b = Uuid::new_v4();
    let generation_b = Uuid::new_v4();
    seed_global_installation(
        &worker,
        organization_b,
        installation_b,
        generation_b,
        release_b,
        "schema-global-two",
    )
    .await;

    let duplicate = sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key,
          lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', $3, 'enabled', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(organization_a)
    .bind("schema-global-two")
    .bind(Uuid::new_v4())
    .bind(actor)
    .execute(&worker)
    .await;
    assert_sqlstate(duplicate, "23505");

    let cross_owner_installation = Uuid::new_v4();
    let cross_owner_generation = Uuid::new_v4();
    let mut cross_owner = worker.begin().await.expect("begin cross-owner seed");
    sqlx::query(
        "INSERT INTO ui_installations
         (id, organization_id, project_id, repository_id, scope, ui_key,
          lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, NULL, NULL, 'global', 'schema-global-cross',
                 'enabled', $3, $4)",
    )
    .bind(cross_owner_installation)
    .bind(organization_b)
    .bind(cross_owner_generation)
    .bind(actor)
    .execute(&mut *cross_owner)
    .await
    .expect("seed cross-owner installation identity");
    sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 1, $3, 'schema-global-cross', 'global')",
    )
    .bind(cross_owner_generation)
    .bind(cross_owner_installation)
    .bind(source_fixture.second_release)
    .execute(&mut *cross_owner)
    .await
    .expect("seed cross-owner generation before deferred check");
    let cross_owner_result = cross_owner.commit().await;
    assert_sqlstate(cross_owner_result, "23000");

    let moved_unreferenced_project =
        sqlx::query("UPDATE projects SET organization_id = $2 WHERE id = $1")
            .bind(unreferenced_project)
            .bind(organization_b)
            .execute(&worker)
            .await;
    moved_unreferenced_project.expect("unreferenced project may change organization");
    let moved_unreferenced_repository =
        sqlx::query("UPDATE repositories SET project_id = $2 WHERE id = $1")
            .bind(unreferenced_repository)
            .bind(project_b)
            .execute(&worker)
            .await;
    moved_unreferenced_repository.expect("unreferenced repository may change project");

    let moved_referenced_project =
        sqlx::query("UPDATE projects SET organization_id = $2 WHERE id = $1")
            .bind(source_fixture.project)
            .bind(organization_b)
            .execute(&worker)
            .await;
    assert_sqlstate(moved_referenced_project, "23000");
    let moved_referenced_repository =
        sqlx::query("UPDATE repositories SET project_id = $2 WHERE id = $1")
            .bind(source_fixture.repository)
            .bind(project_b)
            .execute(&worker)
            .await;
    assert_sqlstate(moved_referenced_repository, "23000");
    let moved_referenced_release =
        sqlx::query("UPDATE releases SET repository_id = $2 WHERE id = $1")
            .bind(source_fixture.second_release)
            .bind(repository_b)
            .execute(&worker)
            .await;
    assert_sqlstate(moved_referenced_release, "23000");

    for (scope, organization_id, project_id, repository_id) in [
        (
            "global",
            Some(organization_a),
            Some(source_fixture.project),
            None,
        ),
        (
            "project",
            Some(organization_a),
            Some(source_fixture.project),
            None,
        ),
        ("repository", None, None, Some(source_fixture.repository)),
    ] {
        let invalid = sqlx::query(
            "INSERT INTO ui_installations
             (id, organization_id, project_id, repository_id, scope, ui_key,
              lifecycle, current_generation_id, created_by)
             VALUES ($1, $2, $3, $4, $5, 'invalid-owner-shape', 'enabled', $6, $7)",
        )
        .bind(Uuid::new_v4())
        .bind(organization_id)
        .bind(project_id)
        .bind(repository_id)
        .bind(scope)
        .bind(Uuid::new_v4())
        .bind(actor)
        .execute(&worker)
        .await;
        assert_sqlstate(invalid, "23514");
    }

    let actor_app = role_pool(&database_url, "hephaestus_app", actor).await;
    let actor_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
        .fetch_one(&actor_app)
        .await
        .expect("dual-member actor global read");
    assert_eq!(actor_visible, 2);
    let actor_a_visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_installations WHERE organization_id = $1")
            .bind(organization_a)
            .fetch_one(&actor_app)
            .await
            .expect("explicit organization A filter");
    let actor_b_visible: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ui_installations WHERE organization_id = $1")
            .bind(organization_b)
            .fetch_one(&actor_app)
            .await
            .expect("explicit organization B filter");
    assert_eq!((actor_a_visible, actor_b_visible), (1, 1));

    let member_a_app = role_pool(&database_url, "hephaestus_app", member_a).await;
    let member_a_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
        .fetch_one(&member_a_app)
        .await
        .expect("organization A member global read");
    assert_eq!(member_a_visible, 1);
    let member_b_app = role_pool(&database_url, "hephaestus_app", member_b).await;
    let member_b_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
        .fetch_one(&member_b_app)
        .await
        .expect("organization B member global read");
    assert_eq!(member_b_visible, 1);
    let outsider_app = role_pool(&database_url, "hephaestus_app", outsider).await;
    let outsider_visible: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
        .fetch_one(&outsider_app)
        .await
        .expect("outsider global read");
    assert_eq!(outsider_visible, 0);
    println!(
        "REAL_GLOBAL_UI_INSTALLATION_SCHEMA=1 migration={max_migration} \
         same_key_two_orgs=1 cross_owner_rejected=1 dual_member_explicit_filter=1 \
         owner_shape=1 organization_rls=1"
    );
}
