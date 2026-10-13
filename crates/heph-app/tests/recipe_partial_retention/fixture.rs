use super::{host, worker::Worker};
use crate::{fixtures, support};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    DeploymentAttemptId, DeploymentIntent, EffectClaim, RetainPartialCreation,
};
use recipe_postgres::PostgresDeploymentRepository;
use sqlx::PgPool;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use tempfile::TempDir;
use uuid::Uuid;
use volume_local::{LocalVolumeConfig, LocalVolumeStore, OwnedVolumeMetadata, VolumeRootOwner};
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingPurpose, OwnedProvisioningClaim, OwnedVolumeRegistration,
    VolumeOwnedProvisioningRepository, VolumeOwnedRegistrationRepository, VolumeRegistration,
};

#[derive(Clone, Copy)]
pub enum BirthMode {
    AllocatedWithdrawal,
    FormatIncomplete,
    ReadyGap,
    NoHostBirth,
}
pub struct Fixture {
    pub pool: PgPool,
    pub seeded: support::Fixture,
    pub recipe: Arc<PostgresDeploymentRepository>,
    pub actor: Arc<PostgresVolumeMetadataRepository>,
    pub worker: Arc<PostgresVolumeMetadataRepository>,
    pub identity: AuthenticatedIdentity,
    pub intent: DeploymentIntent,
    pub target: EffectClaim,
    pub request: BeginOwnedProvisioning,
    pub claim: OwnedProvisioningClaim,
    pub root: TempDir,
    pub tools: TempDir,
    pub owner: Arc<VolumeRootOwner>,
    pub store: Arc<LocalVolumeStore>,
}
impl Fixture {
    pub async fn new(pool: &PgPool, mode: BirthMode) -> Self {
        let super::ledger::Ledger {
            seeded,
            app,
            workers,
            recipe,
            identity,
            intent,
            target,
        } = super::ledger::Ledger::new(pool).await;
        let prepared = recipe
            .prepare_owned_volume_creation(&identity, &target)
            .await
            .expect("durable pre-registration lineage");
        let actor = Arc::new(PostgresVolumeMetadataRepository::new(app));
        let worker = Arc::new(PostgresVolumeMetadataRepository::new(workers));
        let registration = OwnedVolumeRegistration::new(
            VolumeRegistration::new(
                prepared.volume_id,
                prepared.project_id,
                prepared.capacity_bytes,
                prepared.filesystem_uuid,
            )
            .unwrap(),
            prepared.creation,
        );
        let receipt = actor
            .register_owned(&identity, &registration)
            .await
            .expect("actual sealed birth after preparation");
        let root = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let owner =
            Arc::new(VolumeRootOwner::initialize(root.path(), "recipe-partial-host").unwrap());
        host::tools(&tools, mode);
        let purpose = OwnedBackingPurpose::new(
            receipt,
            "recipe-partial-host".into(),
            root.path().into(),
            owner.namespace_id(),
            1,
        )
        .unwrap();
        let request = BeginOwnedProvisioning {
            purpose,
            operation_id: Uuid::new_v4(),
            expected_generation: 0,
        };
        let claim = actor
            .begin_owned_provisioning(&identity, &request)
            .await
            .expect("committed claim before owned backing IO")
            .claim;
        let injected = Arc::new(Worker {
            actual: worker.clone(),
            pool: pool.clone(),
            creator: seeded.maintainer,
            release: seeded.release,
            revoke_recorded: AtomicBool::new(matches!(mode, BirthMode::AllocatedWithdrawal)),
            fail_ready: AtomicBool::new(matches!(mode, BirthMode::ReadyGap)),
        });
        let store = Arc::new(local(
            worker.clone(),
            actor.clone(),
            injected,
            owner.clone(),
            &root,
            &tools,
        ));
        if !matches!(mode, BirthMode::NoHostBirth) {
            assert!(
                store.provision_owned(&identity, &request).await.is_err(),
                "exact interrupted real provider path"
            );
        }
        let claim = worker
            .owned_reconciliation_context(&claim)
            .await
            .unwrap()
            .claim;
        Self {
            pool: pool.clone(),
            seeded,
            recipe,
            actor,
            worker,
            identity,
            intent,
            target,
            request,
            claim,
            root,
            tools,
            owner,
            store,
        }
    }
    pub async fn remove(&self) -> (AuthenticatedIdentity, RetainPartialCreation) {
        sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(self.seeded.consuming_project).bind(self.seeded.outsider.as_uuid()).execute(&self.pool).await.unwrap();
        sqlx::query("DELETE FROM project_maintainers WHERE user_id=$1")
            .bind(self.seeded.maintainer.as_uuid())
            .execute(&self.pool)
            .await
            .unwrap();
        sqlx::query("UPDATE releases SET state='revoked',revoked_at=COALESCE(revoked_at,now()) WHERE id=$1 AND state='published'").bind(self.seeded.release).execute(&self.pool).await.unwrap();
        let identity = support::identity(self.seeded.outsider);
        let snapshot = self
            .recipe
            .inspect(&identity, self.intent.id())
            .await
            .unwrap();
        let removal = fixtures::remove(&identity, self.intent.id(), snapshot.version);
        let command = removal.command;
        let admission = self.recipe.admit_remove(&identity, removal).await.unwrap();
        let request = RetainPartialCreation {
            command,
            target: self.target.clone(),
            operation_id: DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
            expected_deployment_version: admission.snapshot.version,
            expected_resource_version: admission.snapshot.resources[&fixtures::key("data")].version,
        };
        (identity, request)
    }
    pub fn coordinator(&self) -> crate::coordinator::PartialRetentionCoordinator {
        crate::coordinator::PartialRetentionCoordinator::new(
            self.recipe.clone(),
            self.worker.clone(),
            self.store.clone(),
        )
    }
    pub fn format_count(&self) -> usize {
        std::fs::read_to_string(self.tools.path().join("formats"))
            .unwrap_or_default()
            .lines()
            .count()
    }
}

pub fn local(
    legacy: Arc<PostgresVolumeMetadataRepository>,
    actor: Arc<dyn VolumeOwnedProvisioningRepository>,
    worker: Arc<dyn VolumeOwnedProvisioningRepository>,
    owner: Arc<VolumeRootOwner>,
    root: &TempDir,
    tools: &TempDir,
) -> LocalVolumeStore {
    LocalVolumeStore::new(
        legacy,
        LocalVolumeConfig {
            volume_root: root.path().into(),
            transient_runtime_roots: Vec::new(),
            host_id: "recipe-partial-host".into(),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: tools.path().join("mkfs.ext4"),
        },
    )
    .unwrap()
    .with_owned_metadata(OwnedVolumeMetadata::new(actor, worker, owner))
    .unwrap()
}

pub fn declaration(seeded: &support::Fixture) -> DeploymentIntent {
    let source=fixtures::source(seeded,false).replace("resource = \"data\"","resource = \"existing\"")
        .replace("[[resources]]\nkind = \"instance\"","[[resources]]\nkind = \"volume\"\nname = \"existing\"\nsource = { type = \"external\" }\n[[resources]]\nkind = \"instance\"");
    let parsed = recipe_domain::parse_recipe(source.as_bytes()).unwrap();
    let bindings = std::collections::BTreeMap::from([(
        fixtures::key("existing"),
        recipe_domain::ExternalVolume {
            id: runtime_types::VolumeId::from_uuid(seeded.volume),
            capacity_bytes: 16_777_216,
        },
    )]);
    let resolved = parsed
        .resolve(
            &std::collections::BTreeMap::new(),
            &bindings,
            &[fixtures::catalog(seeded)],
        )
        .unwrap();
    DeploymentIntent::new(
        recipe_application::DeploymentId::from_uuid(Uuid::new_v4()).unwrap(),
        forge_domain::ProjectId::from_uuid(seeded.consuming_project),
        recipe_application::DeploymentKey::parse("partial").unwrap(),
        &parsed,
        &resolved,
    )
    .unwrap()
}
