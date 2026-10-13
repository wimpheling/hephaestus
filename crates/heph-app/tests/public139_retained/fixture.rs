use super::native::Native;
use forge_domain::ProjectId;
use heph_runtime::VmProvider;
use identity_domain::{AuthenticatedIdentity, UserId};
use recipe_application::{DeploymentExecutionProfile, EffectClaim, PlatformPolicyObservation};
use recipe_postgres::{
    PostgresDeploymentRepository, PostgresEffectVerificationRecorder,
    PostgresExecutionPreparationRepository, PostgresOwnedReadyRetentionRepository,
};
use release_domain::{NetworkAccess, RuntimePolicy};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::sync::Arc;
use uuid::Uuid;
use volume_postgres::{PostgresReadyRetentionRepository, PostgresVolumeMetadataRepository};
use volume_trait::{OwnedProvisioningContext, OwnedRetainedReuseContext};

pub struct Fixture {
    pub core: Core,
    pub target: EffectClaim,
    pub original: OwnedProvisioningContext,
    pub retained: OwnedRetainedReuseContext,
}

pub struct Core {
    pub admin: PgPool,
    pub app: PgPool,
    pub worker: PgPool,
    pub app_name: String,
    pub project: ProjectId,
    pub identity: AuthenticatedIdentity,
    pub repository: Arc<PostgresDeploymentRepository>,
    pub preparation: Arc<PostgresExecutionPreparationRepository>,
    pub actor: Arc<PostgresOwnedReadyRetentionRepository>,
    pub retention_worker: Arc<PostgresReadyRetentionRepository>,
    pub metadata: Arc<PostgresVolumeMetadataRepository>,
    pub recorder: Arc<PostgresEffectVerificationRecorder>,
    pub native: Native,
    pub actor_metadata: Arc<PostgresVolumeMetadataRepository>,
    pub platform: PlatformPolicyObservation,
}

impl Core {
    pub async fn new(admin: &PgPool) -> Self {
        let migrations = sqlx::migrate!("../../migrations");
        assert_eq!(
            migrations.iter().map(|m| m.version).collect::<Vec<_>>(),
            (1..=143).collect::<Vec<_>>()
        );
        migrations.run(admin).await.unwrap();
        let app_name = format!("heph-public139-paired-{}", Uuid::new_v4());
        let app = role_pool(admin, false, &app_name).await;
        let worker = role_pool(admin, true, "heph-public139-worker").await;
        control_plane_postgres::verify_qualified_invocation_contract(admin, &worker, &app)
            .await
            .unwrap();
        let (project, identity) = principal(admin).await;
        let repository = Arc::new(PostgresDeploymentRepository::new(app.clone()));
        let platform = PlatformPolicyObservation::new(
            RuntimePolicy {
                vcpus: 1,
                memory_mib: 128,
                network: NetworkAccess::Disabled,
            },
            "public139-retained/native-v1".into(),
        )
        .unwrap();
        let preparation = Arc::new(PostgresExecutionPreparationRepository::new(
            app.clone(),
            platform.clone(),
            DeploymentExecutionProfile::RuntimeNamedV1,
        ));
        let actor_metadata = Arc::new(PostgresVolumeMetadataRepository::new(app.clone()));
        let metadata = Arc::new(PostgresVolumeMetadataRepository::new(worker.clone()));
        let recorder = Arc::new(PostgresEffectVerificationRecorder::new(worker.clone()));
        let native = Native::new(actor_metadata.clone(), metadata.clone());
        let scope = native.provider.owner_scope().unwrap();
        let actor = Arc::new(
            PostgresOwnedReadyRetentionRepository::new(
                app.clone(),
                preparation.as_ref().clone(),
                scope.namespace().into(),
                scope.host_id().into(),
            )
            .unwrap(),
        );
        let retention_worker = Arc::new(
            PostgresReadyRetentionRepository::new(
                worker.clone(),
                scope.namespace().into(),
                scope.host_id().into(),
            )
            .unwrap(),
        );
        Self {
            admin: admin.clone(),
            app,
            worker,
            app_name,
            project,
            identity,
            repository,
            preparation,
            actor,
            retention_worker,
            metadata,
            recorder,
            native,
            actor_metadata,
            platform,
        }
    }

    pub async fn close(self) {
        self.app.close().await;
        self.worker.close().await;
    }
}

impl Fixture {
    pub async fn new(admin: &PgPool) -> Self {
        let core = Core::new(admin).await;
        let (target, original) = super::birth::create(&core).await;
        let retained = super::lifecycle::retain(&core, &target, &original).await;
        assert_eq!(retained.retained.original, original);
        Self {
            core,
            target,
            original,
            retained,
        }
    }
    pub async fn close(self) {
        self.core.close().await;
    }
}

async fn role_pool(admin: &PgPool, worker: bool, name: &str) -> PgPool {
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
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("SET lock_timeout = '2s'")
                        .execute(connection)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_with(
            admin
                .connect_options()
                .as_ref()
                .clone()
                .application_name(name),
        )
        .await
        .unwrap()
}

async fn principal(admin: &PgPool) -> (ProjectId, AuthenticatedIdentity) {
    // Exactly the real membership seed shape; no deployment/birth/receipt rows.
    let user = UserId::from_uuid(Uuid::new_v4());
    let owner = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'public139 retained manager'),($2,'public139 organization owner')")
        .bind(user.as_uuid())
        .bind(owner)
        .execute(admin)
        .await
        .unwrap();
    sqlx::query("INSERT INTO organizations(id,name) VALUES($1,'public139 retained fixture')")
        .bind(organization)
        .execute(admin)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO organization_members(organization_id,user_id,role) VALUES($1,$2,'owner')",
    )
    .bind(organization)
    .bind(owner)
    .execute(admin)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO projects(id,organization_id,name) VALUES($1,$2,'public139 retained data')",
    )
    .bind(project)
    .bind(organization)
    .execute(admin)
    .await
    .unwrap();
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(project)
        .bind(user.as_uuid())
        .execute(admin)
        .await
        .unwrap();
    (
        ProjectId::from_uuid(project),
        crate::support::identity(user),
    )
}
