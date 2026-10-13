use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use runtime_types::VolumeId;
use serde_json::json;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use uuid::Uuid;
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{
    OwnedVolumeRegistration, VolumeCreationIdentity, VolumeCreationIdentityWire,
    VolumeCreationOperationId, VolumeOwnershipScopeId, VolumeRegistration,
};

pub fn checkpoint(version: i64) -> Migrator {
    crate::private_migrations::checkpoint(version)
}

pub struct Fixture {
    pub pool: PgPool,
    pub actor: PostgresVolumeMetadataRepository,
    pub worker: PostgresVolumeMetadataRepository,
    pub owner: UserId,
    pub successor: UserId,
    pub project: Uuid,
}

pub fn identity(user: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user,
        "seal-fixture",
        user.as_uuid().to_string(),
        json!({}),
        RequestId::new(),
    )
}

pub async fn setup(pool: &PgPool) -> Fixture {
    checkpoint(110)
        .run(pool)
        .await
        .expect("apply frozen 110 checkpoint");
    seed(pool).await
}

pub async fn seed(pool: &PgPool) -> Fixture {
    let owner = UserId::new();
    let successor = UserId::new();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    for user in [owner, successor] {
        sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'owned-registration')")
            .bind(user.as_uuid())
            .execute(pool)
            .await
            .expect("user");
    }
    sqlx::query("INSERT INTO organizations(id,name) VALUES($1,'owned-registration')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query("INSERT INTO projects(id,organization_id,name) VALUES($1,$2,'owned-registration')")
        .bind(project)
        .bind(organization)
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO organization_members(organization_id,user_id,role) VALUES($1,$2,'member'),($1,$3,'member')")
        .bind(organization).bind(owner.as_uuid()).bind(successor.as_uuid()).execute(pool).await.expect("members");
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2),($1,$3)")
        .bind(project)
        .bind(owner.as_uuid())
        .bind(successor.as_uuid())
        .execute(pool)
        .await
        .expect("managers");
    Fixture {
        pool: pool.clone(),
        actor: PostgresVolumeMetadataRepository::new(role_pool(pool, false).await),
        worker: PostgresVolumeMetadataRepository::new(role_pool(pool, true).await),
        owner,
        successor,
        project,
    }
}

pub async fn role_pool(pool: &PgPool, worker: bool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connection, _| {
            Box::pin(async move {
                if worker {
                    sqlx::query("SET ROLE hephaestus_worker")
                        .execute(connection)
                        .await?;
                } else {
                    sqlx::query("SET ROLE hephaestus_app")
                        .execute(connection)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .expect("role pool")
}

pub fn intent(fixture: &Fixture) -> OwnedVolumeRegistration {
    let creation = VolumeCreationIdentity::try_from(VolumeCreationIdentityWire {
        scope_id: VolumeOwnershipScopeId::from_uuid(Uuid::new_v4()).expect("scope"),
        operation_id: VolumeCreationOperationId::from_uuid(Uuid::new_v4()).expect("operation"),
        command_id: Uuid::new_v4(),
        effect_id: Uuid::new_v4(),
        attempt_id: Uuid::new_v4(),
        actor_id: fixture.owner.as_uuid(),
        request_id: Uuid::new_v4(),
        resource_version: 2,
        generation: 1,
        input_hash: [1; 32],
        intent_hash: [2; 32],
    })
    .expect("correlation");
    OwnedVolumeRegistration::new(
        VolumeRegistration::new(
            VolumeId::new(),
            fixture.project,
            16 * 1024 * 1024,
            Uuid::new_v4(),
        )
        .expect("resource"),
        creation,
    )
}

pub async fn revoke(fixture: &Fixture) {
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(fixture.project)
        .bind(fixture.owner.as_uuid())
        .execute(&fixture.pool)
        .await
        .expect("withdraw manager");
}
