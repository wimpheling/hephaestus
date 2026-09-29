use sqlx::PgPool;

use super::support::Fixture;

pub async fn run(pool: &PgPool, fixture: &Fixture) {
    let unknown: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'unknown', 'repository', $2)")
            .bind(fixture.owner.to_string())
            .bind(fixture.private_repository.to_string())
            .fetch_one(pool)
            .await
            .expect("unknown permission safely denied");
    assert_eq!(unknown, 0);
    let unknown_object: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'can_read', 'unknown', $2)")
            .bind(fixture.owner.to_string())
            .bind(fixture.private_repository.to_string())
            .fetch_one(pool)
            .await
            .expect("unknown object safely denied");
    assert_eq!(unknown_object, 0);
    let app_bypass: bool =
        sqlx::query_scalar("SELECT rolbypassrls FROM pg_roles WHERE rolname = 'hephaestus_app'")
            .fetch_one(pool)
            .await
            .expect("application role flags");
    assert!(!app_bypass);
    let app_owned_protected_tables: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM pg_class
         JOIN pg_roles ON pg_roles.oid = pg_class.relowner
         WHERE pg_class.relname IN
             ('projects', 'repositories', 'build_requests', 'releases',
              'release_artifacts', 'release_agents', 'agent_instances',
              'agent_instance_revisions', 'agent_attachments',
              'agent_updates', 'runs', 'agent_instance_state_volumes')
           AND pg_roles.rolname = 'hephaestus_app'",
    )
    .fetch_one(pool)
    .await
    .expect("protected table ownership");
    assert_eq!(app_owned_protected_tables, 0);
}
