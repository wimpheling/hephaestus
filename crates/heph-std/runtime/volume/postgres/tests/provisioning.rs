//! Isolated actor authorization and native ext4 provisioning evidence.

use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use runtime_types::{RunId, VolumeId};
use serde_json::json;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{borrow::Cow, fs, sync::Arc, time::Duration};
use tempfile::TempDir;
use uuid::Uuid;
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{
    VolumeError, VolumeMetadataRepository, VolumeProvisioningState, VolumeRegistration,
    VolumeResourceRepository, VolumeState, VolumeStore,
};

const MIGRATIONS: Migrator = sqlx::migrate!("../../../../../migrations");

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and e2fsprogs"]
// Keep the audited actor-to-provider sequence together so its ordering is visible.
#[allow(clippy::too_many_lines)]
async fn live_authority_immutable_replay_and_retained_ext4_provisioning(pool: PgPool) {
    checkpoint()
        .run(&pool)
        .await
        .expect("apply provisioning checkpoint");
    let (manager, reader, project) = seed(&pool).await;
    let actor_pool = role_pool(&pool, Role::Actor).await;
    let worker_pool = role_pool(&pool, Role::Worker).await;
    let actor = PostgresVolumeMetadataRepository::new(actor_pool);
    let worker = Arc::new(PostgresVolumeMetadataRepository::new(worker_pool));
    let manager_identity = identity(manager);
    let reader_identity = identity(reader);
    let id = VolumeId::new();
    let filesystem_uuid = Uuid::new_v4();
    let intent =
        VolumeRegistration::new(id, project, 16 * 1024 * 1024, filesystem_uuid).expect("intent");
    assert!(matches!(
        actor.register(&reader_identity, &intent).await,
        Err(VolumeError::PermissionDenied)
    ));
    let denied: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events WHERE request_id=$1 AND decision='deny'",
    )
    .bind(reader_identity.request_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("durable denial audit");
    assert_eq!(denied, 1);
    let registered = actor
        .register(&manager_identity, &intent)
        .await
        .expect("manager registration");
    assert_eq!(
        registered.provisioning_state,
        VolumeProvisioningState::Reserved
    );
    assert_eq!(registered.filesystem_uuid, Some(filesystem_uuid));
    assert_eq!(
        actor
            .register(&manager_identity, &intent)
            .await
            .expect("exact replay"),
        registered
    );
    let conflicting = VolumeRegistration::new(id, project, 32 * 1024 * 1024, filesystem_uuid)
        .expect("different capacity");
    assert!(matches!(
        actor.register(&manager_identity, &conflicting).await,
        Err(VolumeError::IntentConflict)
    ));
    let conflicting = VolumeRegistration::new(id, project, 16 * 1024 * 1024, Uuid::new_v4())
        .expect("different UUID");
    assert!(matches!(
        actor.register(&manager_identity, &conflicting).await,
        Err(VolumeError::IntentConflict)
    ));
    assert!(matches!(
        actor.inspect(&reader_identity, id).await,
        Err(VolumeError::PermissionDenied)
    ));
    assert!(
        matches!(
            actor.inspect(&reader_identity, VolumeId::new()).await,
            Err(VolumeError::PermissionDenied)
        ),
        "existence does not change denied inspection"
    );
    assert!(
        actor
            .list(&reader_identity, project, None, 100)
            .await
            .expect("reader list")
            .is_empty()
    );

    let root = TempDir::new().expect("backing root");
    let mkfs = ["/usr/sbin/mkfs.ext4", "/sbin/mkfs.ext4"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
        .expect("install e2fsprogs");
    let store = LocalVolumeStore::new(
        worker.clone(),
        LocalVolumeConfig {
            volume_root: root.path().to_owned(),
            transient_runtime_roots: Vec::new(),
            host_id: "provider-fixture".into(),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: mkfs.into(),
        },
    )
    .expect("local store");
    store.initialize().await.expect("initialize private root");
    let ready = store.provision(id).await.expect("native ext4 provision");
    assert_eq!(ready.state, VolumeState::Ready);
    assert_eq!(ready.filesystem_uuid, filesystem_uuid);
    let retained = fs::read(&ready.host_path).expect("retain formatted bytes");
    assert_eq!(
        store.provision(id).await.expect("idempotent provision"),
        ready
    );
    assert_eq!(
        fs::read(&ready.host_path).expect("retained backing"),
        retained
    );
    assert!(
        matches!(
            store.acquire(id, RunId::new()).await,
            Err(VolumeError::InvalidState(_))
        ),
        "standalone runtime attachment remains unavailable"
    );
    let inspect = actor
        .inspect(&manager_identity, id)
        .await
        .expect("resource inspection");
    let json = serde_json::to_value(&inspect).expect("safe metadata serialization");
    for private in [
        "host_id",
        "host_path",
        "key_reference",
        "encryption_version",
    ] {
        assert!(json.get(private).is_none());
    }
    assert_eq!(inspect.filesystem_uuid, Some(filesystem_uuid));
    immutable_database_intent(&pool, id).await;

    let unknown_id = VolumeId::new();
    let unknown_intent =
        VolumeRegistration::new(unknown_id, project, 16 * 1024 * 1024, Uuid::new_v4())
            .expect("unknown intent");
    actor
        .register(&manager_identity, &unknown_intent)
        .await
        .expect("second retained resource");
    let reserved = worker
        .reserve_provider(unknown_id, "provider-fixture", root.path())
        .await
        .expect("reserve handles");
    let first = worker
        .claim_provisioning(unknown_id)
        .await
        .expect("claim first generation");
    let second = worker
        .claim_provisioning(unknown_id)
        .await
        .expect("claim replacement generation");
    assert!(matches!(
        worker
            .provisioning_progress(&first, VolumeProvisioningState::Creating)
            .await,
        Err(VolumeError::StaleLease)
    ));
    worker
        .provisioning_progress(&second, VolumeProvisioningState::Uncertain)
        .await
        .expect("fenced uncertainty");
    fs::write(&reserved.host_path, b"unknown retained user bytes").expect("unknown backing");
    assert!(store.provision(unknown_id).await.is_err());
    assert_eq!(
        fs::read(&reserved.host_path).expect("retained unknown bytes"),
        b"unknown retained user bytes"
    );
    let rows = actor
        .list(&manager_identity, project, None, 100)
        .await
        .expect("retained discovery");
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().any(|row| row.id == unknown_id
            && row.provisioning_state == VolumeProvisioningState::Uncertain)
    );
    let page = actor
        .list(&manager_identity, project, None, 1)
        .await
        .expect("bounded first page");
    let tail = actor
        .list(&manager_identity, project, Some(page[0].id), 1)
        .await
        .expect("stable cursor tail");
    assert_eq!(tail.len(), 1);
    assert_ne!(page[0].id, tail[0].id);
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(project)
        .bind(manager.as_uuid())
        .execute(&pool)
        .await
        .expect("revoke live owner authority");
    assert!(
        matches!(
            actor.register(&manager_identity, &intent).await,
            Err(VolumeError::PermissionDenied)
        ),
        "replay checks live authority"
    );
    assert!(matches!(
        actor.inspect(&manager_identity, id).await,
        Err(VolumeError::PermissionDenied)
    ));
    println!(
        "REAL_VOLUME_PROVIDER=1 normal_actor_role=1 durable_denial_audit=1 immutable_intent=1 ext4_uuid_geometry_fsck=1 retained_unknown_bytes=1 metadata_generation_cas=1 standalone_attach_blocked=1"
    );
}

fn checkpoint() -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= 103)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
}

#[derive(Clone, Copy)]
enum Role {
    Actor,
    Worker,
}

async fn role_pool(pool: &PgPool, role: Role) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |connection, _| {
            Box::pin(async move {
                match role {
                    Role::Actor => sqlx::query("SET ROLE hephaestus_app"),
                    Role::Worker => sqlx::query("SET ROLE hephaestus_worker"),
                }
                .execute(connection)
                .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .expect("normal role pool")
}

fn identity(user: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user,
        "https://fixture.example",
        user.to_string(),
        json!({}),
        RequestId::new(),
    )
}

async fn seed(pool: &PgPool) -> (UserId, UserId, Uuid) {
    let manager = UserId::new();
    let reader = UserId::new();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    for user in [manager, reader] {
        sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'volume-provider-fixture')")
            .bind(user.as_uuid())
            .execute(pool)
            .await
            .expect("user");
    }
    sqlx::query("INSERT INTO organizations(id,name) VALUES($1,'volume-provider-fixture')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO projects(id,organization_id,name) VALUES($1,$2,'volume-provider-fixture')",
    )
    .bind(project)
    .bind(organization)
    .execute(pool)
    .await
    .expect("project");
    sqlx::query("INSERT INTO organization_members(organization_id,user_id,role) VALUES($1,$2,'member'),($1,$3,'member')").bind(organization).bind(manager.as_uuid()).bind(reader.as_uuid()).execute(pool).await.expect("reader membership");
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(project)
        .bind(manager.as_uuid())
        .execute(pool)
        .await
        .expect("exact manager");
    (manager, reader, project)
}

async fn immutable_database_intent(pool: &PgPool, id: VolumeId) {
    let errors = [
        sqlx::query("UPDATE agent_instance_state_volumes SET capacity_bytes=capacity_bytes+4096 WHERE id=$1")
            .bind(id.as_uuid()).execute(pool).await.expect_err("immutable capacity"),
        sqlx::query("UPDATE agent_instance_state_volumes SET filesystem_uuid=gen_random_uuid() WHERE id=$1")
            .bind(id.as_uuid()).execute(pool).await.expect_err("immutable UUID"),
        sqlx::query("UPDATE agent_instance_state_volumes SET host_path=host_path||'.changed' WHERE id=$1")
            .bind(id.as_uuid()).execute(pool).await.expect_err("immutable path"),
    ];
    for error in errors {
        assert_eq!(
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("23000")
        );
    }
}
