use crate::support::Fixture;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn source_parity(pool: &PgPool, fixture: &Fixture, legacy: Uuid, standalone: Uuid) {
    let foreign = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,project_id,state,capacity_bytes,filesystem_uuid) VALUES($1,$2,'uninitialized',16777216,gen_random_uuid())")
        .bind(foreign).bind(fixture.project).execute(pool).await.expect("foreign project resource");
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(fixture.project)
        .bind(fixture.outsider.as_uuid())
        .execute(pool)
        .await
        .expect("foreign-only manager");
    for volume in [legacy, standalone, foreign] {
        for user in [
            fixture.maintainer,
            fixture.member,
            fixture.owner,
            fixture.outsider,
        ] {
            let (helper,canonical):(bool,bool)=sqlx::query_as(
                "SELECT private_volume_mount_source_is_live($1,$2),
                  check_permission('user',$1::text,'can_grant_agent_capability','state_volume',$2::text)=1
                  AND check_permission('user',$1::text,'can_attach','state_volume',$2::text)=1",
            ).bind(user.as_uuid()).bind(volume).fetch_one(pool).await.expect("finite direct helper versus canonical permissions");
            assert_eq!(
                helper, canonical,
                "source permission parity for {user}/{volume}"
            );
            let expected =
                user == fixture.maintainer || (volume == foreign && user == fixture.outsider);
            assert_eq!(
                helper, expected,
                "no organization ownership or mere project membership grant"
            );
        }
    }
}

pub async fn immutable(pool: &PgPool, grant: Uuid) {
    for result in [
        sqlx::query(
            "UPDATE agent_instance_volume_mount_grants SET access_mode='read_write' WHERE id=$1",
        )
        .bind(grant)
        .execute(pool)
        .await,
        sqlx::query("DELETE FROM agent_instance_volume_mount_grants WHERE id=$1")
            .bind(grant)
            .execute(pool)
            .await,
    ] {
        assert_eq!(
            result
                .expect_err("immutable authority record")
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("23000")
        );
    }
}

pub async fn binding_snapshot(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT COALESCE(jsonb_agg(to_jsonb(binding) ORDER BY instance_revision_id,slot_key),'[]'::jsonb) FROM agent_instance_revision_volume_bindings binding")
        .fetch_one(pool).await.expect("immutable binding snapshot")
}

pub async fn volume_snapshot(pool: &PgPool, volume: Uuid) -> serde_json::Value {
    sqlx::query_scalar(
        "SELECT to_jsonb(volume) FROM agent_instance_state_volumes volume WHERE id=$1",
    )
    .bind(volume)
    .fetch_one(pool)
    .await
    .expect("old provider metadata snapshot")
}

pub async fn historical_checksums(pool: &PgPool) -> Vec<(i64, Vec<u8>)> {
    sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=104 ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .expect("historical migration checksums")
}

pub async fn seed_active_lease(pool: &PgPool, fixture: &Fixture) -> Uuid {
    let lease = Uuid::new_v4();
    sqlx::query(
        "UPDATE agent_instance_state_volumes SET lease_generation=19,state='attached' WHERE id=$1",
    )
    .bind(fixture.volume)
    .execute(pool)
    .await
    .expect("existing attached volume fence");
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES($1,$2,$3,$4,'host',19,'active',now()+interval '1 hour')")
        .bind(lease).bind(fixture.volume).bind(fixture.instance).bind(fixture.run).execute(pool).await.expect("active legacy lease before105");
    sqlx::query("UPDATE runs SET volume_id=$2,lease_id=$3,lease_fencing_token=19 WHERE id=$1")
        .bind(fixture.run)
        .bind(fixture.volume)
        .bind(lease)
        .execute(pool)
        .await
        .expect("legacy scalar run evidence");
    lease
}

pub async fn lease_snapshot(pool: &PgPool, lease: Uuid) -> serde_json::Value {
    sqlx::query_scalar("SELECT to_jsonb(lease) FROM agent_instance_volume_leases lease WHERE id=$1")
        .bind(lease)
        .fetch_one(pool)
        .await
        .expect("legacy lease snapshot")
}

pub async fn run_evidence(pool: &PgPool, run: Uuid) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('id',id,'volume_id',volume_id,'lease_id',lease_id,'fence',lease_fencing_token,'state',state) FROM runs WHERE id=$1")
        .bind(run).fetch_one(pool).await.expect("legacy scalar fenced run snapshot")
}

pub async fn legacy_read_permission(pool: &PgPool, fixture: &Fixture) -> i32 {
    sqlx::query_scalar("SELECT check_permission('user',$1,'can_read','state_volume',$2)")
        .bind(fixture.member.to_string())
        .bind(fixture.volume.to_string())
        .fetch_one(pool)
        .await
        .expect("legacy instance-derived reader")
}
