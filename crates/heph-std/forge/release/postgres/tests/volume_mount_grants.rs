//! Real actor and worker proof for exact immutable typed mount authority.

use authz_postgres::PostgresMelangeAuthorizer;
use capability_domain::AuthorityHash;
use release_postgres::ReleaseService;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use runtime_types::RunId;
use sqlx::{PgPool, migrate::Migrator};
use std::{borrow::Cow, sync::Arc};
use volume_domain::{
    VolumeAccessMode, VolumeMountGrantId, VolumeMountRevocationId, VolumeMountScope,
};
use volume_trait::{VolumeError, VolumeMountGrantRepository};

#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
// Share the authorization fixture; its unrelated legacy matrix fields are unused.
#[path = "volume_mount_grants/fixture.rs"]
mod fixture;
#[path = "volume_mount_grants/parity.rs"]
mod parity;
#[allow(dead_code)]
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
mod support;

const MIGRATIONS: Migrator = sqlx::migrate!("../../../../../migrations");

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn exact_mount_authority_is_live_revocable_and_revision_bound(pool: PgPool) {
    checkpoint(104)
        .run(&pool)
        .await
        .expect("previous checkpoint104");
    let seeded = seed::seed(&pool).await;
    let before: Vec<u8> =
        sqlx::query_scalar("SELECT runtime_contract_hash FROM release_agents WHERE id=$1")
            .bind(seeded.release_agent)
            .fetch_one(&pool)
            .await
            .unwrap();
    let legacy = fixture::legacy(&pool, &seeded).await;
    let explicit = fixture::explicit(&pool, &seeded).await;
    let lease = parity::seed_active_lease(&pool, &seeded).await;
    let old_lease = parity::lease_snapshot(&pool, lease).await;
    let old_run = parity::run_evidence(&pool, seeded.run).await;
    let old_permission = parity::legacy_read_permission(&pool, &seeded).await;
    let old_bindings = parity::binding_snapshot(&pool).await;
    let old_checksums = parity::historical_checksums(&pool).await;
    let old_volume = parity::volume_snapshot(&pool, seeded.volume).await;
    checkpoint(105)
        .run(&pool)
        .await
        .expect("upgrade populated schema104 to105");
    assert_eq!(parity::binding_snapshot(&pool).await, old_bindings);
    assert_eq!(parity::historical_checksums(&pool).await, old_checksums);
    assert_eq!(
        parity::volume_snapshot(&pool, seeded.volume).await,
        old_volume
    );
    assert_eq!(parity::lease_snapshot(&pool, lease).await, old_lease);
    assert_eq!(parity::run_evidence(&pool, seeded.run).await, old_run);
    assert_eq!(
        parity::legacy_read_permission(&pool, &seeded).await,
        old_permission
    );
    assert_eq!(old_permission, 1);
    parity::source_parity(
        &pool,
        &seeded,
        legacy.0.volume_id().as_uuid(),
        explicit.0.volume_id().as_uuid(),
    )
    .await;
    let actor_pool = fixture::role_pool(&pool, fixture::Role::Actor).await;
    let worker_pool = fixture::role_pool(&pool, fixture::Role::Worker).await;
    let service = ReleaseService::new(actor_pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let runtime = PgRuntimeSessionRepository::new(worker_pool.clone());
    for (scope, run) in [legacy, explicit] {
        exercise(&pool, &seeded, &service, &runtime, scope, run).await;
    }
    let after: Vec<u8> =
        sqlx::query_scalar("SELECT runtime_contract_hash FROM release_agents WHERE id=$1")
            .bind(seeded.release_agent)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(before, after, "published legacy hash is unchanged");
    actor_pool.close().await;
    worker_pool.close().await;
    println!(
        "REAL_VOLUME_MOUNT_GRANTS=1 actor_role=1 exact_normal_revision_mode=1 live_source_parity=1 no_binding_grant=1 role_withdraw_restore=1 permanent_revocation=1 frozen_hash_preserved=1 upgrade_104_to105=1 old_checksums_bindings_metadata_preserved=1 active_lease_run_fence_legacy_permissions_preserved=1"
    );
}

async fn exercise(
    pool: &PgPool,
    fixture: &support::Fixture,
    service: &ReleaseService,
    runtime: &PgRuntimeSessionRepository,
    scope: VolumeMountScope,
    run: RunId,
) {
    let grant = creation_and_replay(pool, fixture, service, runtime, &scope, run).await;
    exact_ceilings(pool, fixture, runtime, &scope, run).await;
    source_withdrawal_and_restore(pool, fixture, runtime, &scope, run).await;
    permanent_revocation(fixture, service, runtime, &scope, run, grant).await;
}

async fn creation_and_replay(
    pool: &PgPool,
    fixture: &support::Fixture,
    service: &ReleaseService,
    runtime: &PgRuntimeSessionRepository,
    scope: &VolumeMountScope,
    run: RunId,
) -> VolumeMountGrantId {
    let creator = support::identity(fixture.maintainer);
    let grant = VolumeMountGrantId::new();
    assert!(
        !runtime.volume_mount_authorized(run, scope).await.unwrap(),
        "binding alone grants nothing"
    );
    let coarse: i32 = sqlx::query_scalar(
        "SELECT check_permission('agent_instance',$1,'agent_attach','state_volume',$2)",
    )
    .bind(scope.instance_id().to_string())
    .bind(scope.volume_id().to_string())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(coarse, 0);
    let denied = support::identity(fixture.member);
    assert!(matches!(
        service.grant_volume_mount(&denied, grant, scope).await,
        Err(VolumeError::PermissionDenied)
    ));
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events WHERE request_id=$1 AND decision='deny'",
    )
    .bind(denied.request_id.as_uuid())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(audits, 1, "denial remains committed");
    let stored = service
        .grant_volume_mount(&creator, grant, scope)
        .await
        .expect("exact two-sided actor grant");
    assert_eq!(
        service
            .grant_volume_mount(&creator, grant, scope)
            .await
            .unwrap(),
        stored,
        "immutable replay"
    );
    assert!(runtime.volume_mount_authorized(run, scope).await.unwrap());
    parity::immutable(pool, grant.as_uuid()).await;
    grant
}

async fn exact_ceilings(
    pool: &PgPool,
    fixture: &support::Fixture,
    runtime: &PgRuntimeSessionRepository,
    scope: &VolumeMountScope,
    run: RunId,
) {
    let changed = VolumeMountScope::new(
        scope.instance_id(),
        scope.revision_id(),
        scope.release_agent_id(),
        scope.slot().clone(),
        scope.volume_id(),
        match scope.access_mode() {
            VolumeAccessMode::ReadOnly => VolumeAccessMode::ReadWrite,
            VolumeAccessMode::ReadWrite => VolumeAccessMode::ReadOnly,
        },
        scope.release_contract_hash(),
    )
    .unwrap();
    assert!(
        !runtime
            .volume_mount_authorized(run, &changed)
            .await
            .unwrap(),
        "exact mode required"
    );
    let changed = VolumeMountScope::new(
        scope.instance_id(),
        scope.revision_id(),
        scope.release_agent_id(),
        scope.slot().clone(),
        scope.volume_id(),
        scope.access_mode(),
        AuthorityHash::from_bytes([77; 32]),
    )
    .unwrap();
    assert!(
        !runtime
            .volume_mount_authorized(run, &changed)
            .await
            .unwrap(),
        "exact frozen hash required"
    );
    assert!(
        !runtime
            .volume_mount_authorized(RunId::from_uuid(fixture.run), scope)
            .await
            .unwrap(),
        "typed update run unsupported"
    );
    sqlx::query("UPDATE agent_instances SET state='paused_unknown_state' WHERE id=$1")
        .bind(scope.instance_id().as_uuid())
        .execute(pool)
        .await
        .unwrap();
    assert!(
        !runtime.volume_mount_authorized(run, scope).await.unwrap(),
        "paused consumer denied"
    );
    sqlx::query("UPDATE agent_instances SET state='active' WHERE id=$1")
        .bind(scope.instance_id().as_uuid())
        .execute(pool)
        .await
        .unwrap();
}

async fn source_withdrawal_and_restore(
    pool: &PgPool,
    fixture: &support::Fixture,
    runtime: &PgRuntimeSessionRepository,
    scope: &VolumeMountScope,
    run: RunId,
) {
    let creator = support::identity(fixture.maintainer);
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(fixture.consuming_project)
        .bind(creator.user_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    assert!(
        !runtime.volume_mount_authorized(run, scope).await.unwrap(),
        "creator source authority must stay live"
    );
    let canonical:bool=sqlx::query_scalar("SELECT private_volume_mount_source_is_live($1,$2)= (check_permission('user',$1::text,'can_grant_agent_capability','state_volume',$2::text)=1 AND check_permission('user',$1::text,'can_attach','state_volume',$2::text)=1)")
        .bind(creator.user_id.as_uuid()).bind(scope.volume_id().as_uuid()).fetch_one(pool).await.unwrap();
    assert!(canonical, "withdrawal parity");
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(fixture.consuming_project)
        .bind(creator.user_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    assert!(
        runtime.volume_mount_authorized(run, scope).await.unwrap(),
        "role restore before explicit revoke"
    );
}

async fn permanent_revocation(
    fixture: &support::Fixture,
    service: &ReleaseService,
    runtime: &PgRuntimeSessionRepository,
    scope: &VolumeMountScope,
    run: RunId,
    grant: VolumeMountGrantId,
) {
    let creator = support::identity(fixture.maintainer);
    let revoke = VolumeMountRevocationId::new();
    let record = service
        .revoke_volume_mount(&creator, revoke, grant)
        .await
        .expect("authorized permanent withdrawal");
    assert_eq!(
        service
            .revoke_volume_mount(&creator, revoke, grant)
            .await
            .unwrap(),
        record
    );
    assert!(!runtime.volume_mount_authorized(run, scope).await.unwrap());
    assert!(
        service
            .grant_volume_mount(&creator, grant, scope)
            .await
            .is_err(),
        "revoked stamp replay rejected"
    );
    assert!(
        service
            .grant_volume_mount(&creator, VolumeMountGrantId::new(), scope)
            .await
            .is_err(),
        "same revision cannot silently regrant"
    );
}

fn checkpoint(version: i64) -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ..MIGRATIONS
    }
}
