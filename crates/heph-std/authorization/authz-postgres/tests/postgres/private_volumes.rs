use authz_postgres::begin_actor_transaction;
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{Fixture, identity};

pub async fn run(pool: &PgPool, fixture: &Fixture) {
    let volume = Uuid::new_v4();
    let occurrence = Uuid::new_v4();
    create_as_manager(pool, fixture, volume, occurrence).await;
    legacy_insert_returning(pool, fixture).await;
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(fixture.consuming_project)
        .bind(fixture.outsider.as_uuid())
        .execute(pool)
        .await
        .expect("foreign project manager fixture");
    permission_cases(pool, fixture, volume).await;
    rls_cases(pool, fixture, volume).await;
    immutable_owner_cases(pool, fixture, volume).await;
    binding_cases(pool, fixture, volume).await;
    let event_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events WHERE occurrence_id = $1
         AND scope_kind = 'project' AND scope_id = $2
         AND aggregate_type = 'project' AND event_type = 'project.changed'",
    )
    .bind(occurrence)
    .bind(fixture.project)
    .fetch_one(pool)
    .await
    .expect("committed owning project event");
    assert_eq!(event_count, 1);
    let null_scopes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM application_events WHERE occurrence_id = $1 AND scope_id IS NULL",
    )
    .bind(occurrence)
    .fetch_one(pool)
    .await
    .expect("standalone event scope integrity");
    assert_eq!(null_scopes, 0);
    let protected: bool = sqlx::query_scalar(
        "SELECT relrowsecurity AND relforcerowsecurity FROM pg_class
         WHERE oid = 'agent_instance_revision_volume_bindings'::regclass",
    )
    .fetch_one(pool)
    .await
    .expect("forced binding RLS");
    assert!(protected);
    let legacy_index: bool =
        sqlx::query_scalar("SELECT to_regclass('one_active_instance_run_lease') IS NOT NULL")
            .fetch_one(pool)
            .await
            .expect("legacy single-run lease ceiling remains");
    assert!(legacy_index);
}

async fn create_as_manager(pool: &PgPool, fixture: &Fixture, volume: Uuid, occurrence: Uuid) {
    let mut tx = begin_actor_transaction(pool, &identity(fixture.maintainer))
        .await
        .expect("manager actor context");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *tx)
        .await
        .expect("normal actor role");
    sqlx::query("SELECT set_config('hephaestus.occurrence_id', $1, true)")
        .bind(occurrence.to_string())
        .execute(&mut *tx)
        .await
        .expect("event occurrence");
    let created: Uuid = sqlx::query_scalar(
        "INSERT INTO agent_instance_state_volumes (id, project_id, state, capacity_bytes, filesystem_uuid)
         VALUES ($1, $2, 'uninitialized', 16777216, gen_random_uuid()) RETURNING id",
    )
    .bind(volume)
    .bind(fixture.project)
    .fetch_one(&mut *tx)
    .await
    .expect("standalone creation under forced RLS");
    assert_eq!(created, volume);
    tx.commit().await.expect("commit standalone resource");
}

async fn legacy_insert_returning(pool: &PgPool, fixture: &Fixture) {
    let instance = Uuid::new_v4();
    let volume = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_instances (id, project_id, family_id, name, state)
         SELECT $1, project_id, family_id, $1::text, 'active'
         FROM agent_instances WHERE id = $2",
    )
    .bind(instance)
    .bind(fixture.instance)
    .execute(pool)
    .await
    .expect("legacy origin fixture");
    let mut tx = begin_actor_transaction(pool, &identity(fixture.maintainer))
        .await
        .expect("legacy manager actor context");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *tx)
        .await
        .expect("normal actor role");
    let projected: (Uuid, Uuid) = sqlx::query_as(
        "INSERT INTO agent_instance_state_volumes (id, instance_id, state, capacity_bytes)
         VALUES ($1, $2, 'uninitialized', 16777216) RETURNING id, project_id",
    )
    .bind(volume)
    .bind(instance)
    .fetch_one(&mut *tx)
    .await
    .expect("legacy INSERT RETURNING fills and exposes stable owner");
    assert_eq!(projected, (volume, fixture.consuming_project));
    tx.rollback().await.expect("rollback legacy insert probe");
}

async fn permission_cases(pool: &PgPool, fixture: &Fixture, volume: Uuid) {
    for (user, expected) in [
        (fixture.maintainer, 1),
        (fixture.member, 0),
        (fixture.owner, 0),
        (fixture.outsider, 0),
    ] {
        for permission in [
            "can_read",
            "can_manage",
            "can_attach",
            "can_restore",
            "can_grant_agent_capability",
        ] {
            let decision: i32 =
                sqlx::query_scalar("SELECT check_permission('user', $1, $2, 'state_volume', $3)")
                    .bind(user.to_string())
                    .bind(permission)
                    .bind(volume.to_string())
                    .fetch_one(pool)
                    .await
                    .expect("exact standalone permission");
            assert_eq!(decision, expected, "{permission} for {user}");
        }
    }
    let legacy_member_read: i32 =
        sqlx::query_scalar("SELECT check_permission('user', $1, 'can_read', 'state_volume', $2)")
            .bind(fixture.member.to_string())
            .bind(fixture.volume.to_string())
            .fetch_one(pool)
            .await
            .expect("legacy instance-derived reading remains");
    assert_eq!(legacy_member_read, 1);
    let no_ambient_attach: i32 = sqlx::query_scalar(
        "SELECT check_permission('agent_instance', $1, 'agent_attach', 'state_volume', $2)",
    )
    .bind(fixture.instance.to_string())
    .bind(volume.to_string())
    .fetch_one(pool)
    .await
    .expect("project ownership creates no workload grant");
    assert_eq!(no_ambient_attach, 0);
}

async fn rls_cases(pool: &PgPool, fixture: &Fixture, volume: Uuid) {
    for user in [fixture.member, fixture.owner, fixture.outsider] {
        let mut tx = begin_actor_transaction(pool, &identity(user))
            .await
            .expect("actor context");
        sqlx::query("SET LOCAL ROLE hephaestus_app")
            .execute(&mut *tx)
            .await
            .expect("normal actor role");
        let hidden: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM agent_instance_state_volumes WHERE id = $1")
                .bind(volume)
                .fetch_optional(&mut *tx)
                .await
                .expect("standalone denied read");
        assert!(hidden.is_none());
        let forbidden = sqlx::query(
            "INSERT INTO agent_instance_state_volumes (id, project_id, state, capacity_bytes, filesystem_uuid)
             VALUES ($1, $2, 'uninitialized', 16777216, gen_random_uuid())",
        )
        .bind(Uuid::new_v4())
        .bind(fixture.project)
        .execute(&mut *tx)
        .await;
        assert_eq!(
            forbidden
                .expect_err("reader cannot create a volume")
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("42501")
        );
        tx.rollback().await.expect("rollback denied transaction");
    }
}

async fn immutable_owner_cases(pool: &PgPool, fixture: &Fixture, volume: Uuid) {
    for project_change in [true, false] {
        let mut tx = begin_actor_transaction(pool, &identity(fixture.maintainer))
            .await
            .expect("manager context");
        sqlx::query("SET LOCAL ROLE hephaestus_app")
            .execute(&mut *tx)
            .await
            .expect("normal actor role");
        let attempted = if project_change {
            sqlx::query("UPDATE agent_instance_state_volumes SET project_id = $2 WHERE id = $1")
                .bind(volume)
                .bind(fixture.consuming_project)
                .execute(&mut *tx)
                .await
        } else {
            sqlx::query("UPDATE agent_instance_state_volumes SET instance_id = $2 WHERE id = $1")
                .bind(volume)
                .bind(fixture.instance)
                .execute(&mut *tx)
                .await
        };
        let failure = attempted.expect_err("owner and origin cannot change");
        assert_eq!(
            failure
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("23000")
        );
        tx.rollback()
            .await
            .expect("rollback attempted reassignment");
    }
}

async fn binding_cases(pool: &PgPool, fixture: &Fixture, foreign_volume: Uuid) {
    sqlx::query("UPDATE agent_instances SET state_volume_id = $2 WHERE id = $1")
        .bind(fixture.instance)
        .bind(fixture.volume)
        .execute(pool)
        .await
        .expect("prove legacy pointer");
    let revision: Uuid =
        sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id = $1")
            .bind(fixture.instance)
            .fetch_one(pool)
            .await
            .expect("exact immutable revision");
    let mut tx = begin_actor_transaction(pool, &identity(fixture.maintainer))
        .await
        .expect("manager context");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *tx)
        .await
        .expect("normal actor role");
    sqlx::query(
        "INSERT INTO agent_instance_revision_volume_bindings
         (instance_revision_id, instance_id, project_id, release_agent_id, slot_key,
          volume_id, access_mode, guest_path, slot_required, minimum_capacity_bytes, provenance, created_by)
         VALUES ($1, $2, $3, $4, 'state', $5, 'read_write', '/var/lib/hephaestus', true, 1, 'legacy_state', $6)",
    ).bind(revision).bind(fixture.instance).bind(fixture.consuming_project).bind(fixture.release_agent)
        .bind(fixture.volume).bind(fixture.maintainer.as_uuid()).execute(&mut *tx).await.expect("exact legacy binding under actor RLS");
    tx.commit().await.expect("commit binding evidence");
    for (selected, mode) in [
        (fixture.volume, "read_only"),
        (foreign_volume, "read_write"),
    ] {
        let denied = sqlx::query(
            "INSERT INTO agent_instance_revision_volume_bindings
             (instance_revision_id, instance_id, project_id, release_agent_id, slot_key,
              volume_id, access_mode, guest_path, slot_required, minimum_capacity_bytes, provenance)
             VALUES ($1, $2, $3, $4, 'state', $5, $6, '/var/lib/hephaestus', true, 1, 'legacy_state')",
        ).bind(revision).bind(fixture.instance).bind(fixture.consuming_project).bind(fixture.release_agent)
            .bind(selected).bind(mode).execute(pool).await.expect_err("exact mode and project integrity");
        assert_eq!(
            denied
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("23000")
        );
    }
    for attempted in [
        sqlx::query("UPDATE agent_instance_revision_volume_bindings SET access_mode = 'read_only' WHERE instance_revision_id = $1")
            .bind(revision).execute(pool).await,
        sqlx::query("DELETE FROM agent_instance_revision_volume_bindings WHERE instance_revision_id = $1")
            .bind(revision).execute(pool).await,
    ] {
        let denied = attempted.expect_err("binding records are immutable");
        assert_eq!(denied.as_database_error().and_then(sqlx::error::DatabaseError::code).as_deref(), Some("23000"));
    }
}
