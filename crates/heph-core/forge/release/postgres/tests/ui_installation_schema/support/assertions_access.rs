use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{borrow::Cow, time::Duration};
use uuid::Uuid;

use super::fixture::Fixture;

pub async fn assert_worker_grants(pool: &PgPool) {
    for table in [
        "ui_installations",
        "ui_installation_generations",
        "ui_installation_bindings",
        "ui_installation_commands",
    ] {
        let can_insert: bool =
            sqlx::query_scalar("SELECT has_table_privilege(current_user, $1, 'INSERT')")
                .bind(table)
                .fetch_one(pool)
                .await
                .expect("read worker INSERT privilege");
        assert!(can_insert, "worker can append to {table}");
        let can_update: bool =
            sqlx::query_scalar("SELECT has_table_privilege(current_user, $1, 'UPDATE')")
                .bind(table)
                .fetch_one(pool)
                .await
                .expect("read worker UPDATE privilege");
        let can_delete: bool =
            sqlx::query_scalar("SELECT has_table_privilege(current_user, $1, 'DELETE')")
                .bind(table)
                .fetch_one(pool)
                .await
                .expect("read worker DELETE privilege");
        assert_eq!(
            can_update,
            table == "ui_installations",
            "worker UPDATE privilege for {table}"
        );
        assert!(!can_delete, "worker cannot delete {table}");
    }
}

pub async fn assert_installation_identity_is_immutable(pool: &PgPool, fixture: &Fixture) {
    let id_update = sqlx::query("UPDATE ui_installations SET id = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(Uuid::new_v4())
        .execute(pool)
        .await;
    assert_sqlstate(id_update, "23000");

    let creator_update = sqlx::query("UPDATE ui_installations SET created_by = $2 WHERE id = $1")
        .bind(fixture.installation)
        .bind(Uuid::new_v4())
        .execute(pool)
        .await;
    assert_sqlstate(creator_update, "23000");

    let created_at_update = sqlx::query(
        "UPDATE ui_installations
         SET created_at = created_at + interval '1 second'
         WHERE id = $1",
    )
    .bind(fixture.installation)
    .execute(pool)
    .await;
    assert_sqlstate(created_at_update, "23000");
}

pub async fn assert_composite_fks(pool: &PgPool, fixture: &Fixture) {
    let bad_generation_release_ui = sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 99, $3, $4, 'project')",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.installation)
    .bind(fixture.second_release)
    .bind(fixture.ui_key)
    .execute(pool)
    .await;
    assert_sqlstate(bad_generation_release_ui, "23503");

    let bad_scope_key = sqlx::query(
        "INSERT INTO ui_installation_generations
         (id, installation_id, generation_no, release_id, ui_key, ui_scope)
         VALUES ($1, $2, 99, $3, $4, 'repository')",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.repository_two_installation)
    .bind(fixture.release)
    .bind(fixture.ui_key)
    .execute(pool)
    .await;
    assert_sqlstate(bad_scope_key, "23503");

    let bad_current_generation =
        sqlx::query("UPDATE ui_installations SET current_generation_id = $2 WHERE id = $1")
            .bind(fixture.repository_two_installation)
            .bind(fixture.generation)
            .execute(pool)
            .await;
    assert_sqlstate(bad_current_generation, "23503");

    // Every referenced release/UI/gateway/agent exists, but the generation's
    // immutable release/UI tuple does not match the supplied binding.
    let bad_binding_release_ui = sqlx::query(
        "INSERT INTO ui_installation_bindings
         (installation_id, generation_id, binding_kind, binding_key, release_id,
          ui_key, gateway_id, gateway_revision_id, release_agent_id, gateway_name,
          method, route, exposure)
         VALUES ($1, $2, 'api', 'second-release-ui', $3, 'schema-ui-two', $4, $5, $6,
                 'ui-gateway-two', 'GET', '/ui-two', 'heph_authenticated')",
    )
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.second_release)
    .bind(fixture.second_gateway)
    .bind(fixture.second_gateway_revision)
    .bind(fixture.second_release_agent)
    .execute(pool)
    .await;
    assert_sqlstate(bad_binding_release_ui, "23503");

    // The release agent exists, but it does not match the exact gateway
    // revision identity tuple.
    let bad_binding_agent = sqlx::query(
        "INSERT INTO ui_installation_bindings
         (installation_id, generation_id, binding_kind, binding_key, release_id,
          ui_key, gateway_id, gateway_revision_id, release_agent_id, gateway_name,
          method, route, exposure)
         VALUES ($1, $2, 'api', 'wrong-agent', $3, $4, $5, $6, $7,
                 'ui-gateway', 'GET', '/ui', 'heph_authenticated')",
    )
    .bind(fixture.installation)
    .bind(fixture.generation)
    .bind(fixture.release)
    .bind(fixture.ui_key)
    .bind(fixture.gateway)
    .bind(fixture.gateway_revision)
    .bind(fixture.second_release_agent)
    .execute(pool)
    .await;
    assert_sqlstate(bad_binding_agent, "23503");
}

pub async fn assert_immutability_and_removed_terminal(pool: &PgPool, fixture: &Fixture) {
    let generation_update =
        sqlx::query("UPDATE ui_installation_generations SET generation_no = 2 WHERE id = $1")
            .bind(fixture.generation)
            .execute(pool)
            .await;
    assert_sqlstate(generation_update, "42501");

    let binding_delete = sqlx::query(
        "DELETE FROM ui_installation_bindings
         WHERE generation_id = $1 AND binding_kind = 'api' AND binding_key = $2",
    )
    .bind(fixture.generation)
    .bind(fixture.binding_key)
    .execute(pool)
    .await;
    assert_sqlstate(binding_delete, "42501");

    let command_update =
        sqlx::query("UPDATE ui_installation_commands SET input_hash = $2 WHERE command_key = $1")
            .bind(fixture.command.as_slice())
            .bind(vec![8_u8; 32])
            .execute(pool)
            .await;
    assert_sqlstate(command_update, "42501");

    let terminal_update =
        sqlx::query("UPDATE ui_installations SET lifecycle = 'disabled' WHERE id = $1")
            .bind(fixture.installation)
            .execute(pool)
            .await;
    assert_sqlstate(terminal_update, "23000");
}

pub async fn assert_application_rls(database_url: &str, fixture: &Fixture) {
    let owner = role_pool(database_url, "hephaestus_app", fixture.actor).await;
    let forced_rls: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM pg_class
         WHERE relname IN (
             'ui_installations', 'ui_installation_generations',
             'ui_installation_bindings', 'ui_installation_commands'
         )
         AND relrowsecurity AND relforcerowsecurity",
    )
    .fetch_one(&owner)
    .await
    .expect("read UI installation RLS flags");
    assert_eq!(forced_rls, 4, "all UI installation tables use FORCE RLS");

    let visible: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM ui_installations WHERE id = $1),
            (SELECT count(*) FROM ui_installation_generations WHERE installation_id = $1),
            (SELECT count(*) FROM ui_installation_bindings WHERE installation_id = $1),
            (SELECT count(*) FROM ui_installation_commands WHERE installation_id = $1)",
    )
    .bind(fixture.installation)
    .fetch_one(&owner)
    .await
    .expect("authorized application read");
    assert_eq!(
        visible,
        (1, 1, 1, 1),
        "authorized owner can inspect retained history"
    );

    let outsider = role_pool(database_url, "hephaestus_app", fixture.outsider).await;
    let hidden: i64 = sqlx::query_scalar("SELECT count(*) FROM ui_installations")
        .fetch_one(&outsider)
        .await
        .expect("outsider installation read");
    assert_eq!(hidden, 0);
    let hidden_children: (i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT count(*) FROM ui_installation_generations),
            (SELECT count(*) FROM ui_installation_bindings),
            (SELECT count(*) FROM ui_installation_commands)",
    )
    .fetch_one(&outsider)
    .await
    .expect("outsider child installation reads");
    assert_eq!(hidden_children, (0, 0, 0));

    let denied_insert = sqlx::query(
        "INSERT INTO ui_installations
         (id, project_id, scope, ui_key, lifecycle, current_generation_id, created_by)
         VALUES ($1, $2, 'project', 'app-write', 'enabled', $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(fixture.project)
    .bind(Uuid::new_v4())
    .bind(fixture.actor)
    .execute(&owner)
    .await;
    assert_sqlstate(denied_insert, "42501");

    let denied_update = sqlx::query("UPDATE ui_installations SET updated_at = now() WHERE id = $1")
        .bind(fixture.installation)
        .execute(&owner)
        .await;
    assert_sqlstate(denied_update, "42501");
}

pub async fn admin_pool(database_url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await
        .expect("connect migration bootstrap pool")
}

pub async fn role_pool(database_url: &str, role: &str, actor: Uuid) -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect({
            let role = role.to_owned();
            move |connection, _metadata| {
                let role = role.clone();
                Box::pin(async move {
                    sqlx::query("SELECT set_config('role', $1, false)")
                        .bind(role)
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, false)")
                        .bind(actor.to_string())
                        .execute(&mut *connection)
                        .await
                        .map(|_| ())
                })
            }
        })
        .connect(database_url)
        .await
        .expect("connect role pool")
}

pub async fn assert_role(pool: &PgPool, expected: &str, superuser: bool, bypassrls: bool) {
    let row: (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, rolsuper, rolbypassrls
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(pool)
    .await
    .expect("read PostgreSQL role identity");
    assert_eq!(row, (expected.to_owned(), superuser, bypassrls));
}

pub fn assert_sqlstate<T>(result: Result<T, sqlx::Error>, expected: &str) {
    let error = match result {
        Ok(_) => panic!("expected PostgreSQL error {expected}"),
        Err(error) => error,
    };
    let code = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(Cow::into_owned);
    assert_eq!(code.as_deref(), Some(expected));
}
