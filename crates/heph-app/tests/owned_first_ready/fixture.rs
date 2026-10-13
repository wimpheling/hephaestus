use crate::{fixtures, instance_fixture::Harness};
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, DeploymentExecutionPreparationRepository, EffectClaim,
    ResourceAction, ResourceEffect,
};
use sqlx::PgPool;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc, time::Duration};
use tempfile::TempDir;
use uuid::Uuid;
use volume_local::{LocalVolumeConfig, LocalVolumeStore, OwnedVolumeMetadata, VolumeRootOwner};
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::{
    BeginOwnedProvisioning, OwnedProvisioningExpectation, OwnedVolumeRegistration,
    VolumeOwnedRegistrationRepository, VolumeRegistration,
};

pub struct Fixture {
    pub h: Harness,
    pub root: TempDir,
    pub tools: TempDir,
    pub owner: Arc<VolumeRootOwner>,
    pub actor: Arc<PostgresVolumeMetadataRepository>,
    pub worker: Arc<PostgresVolumeMetadataRepository>,
    pub local: LocalVolumeStore,
}
impl Fixture {
    pub async fn new(pool: &PgPool) -> Self {
        let h = Harness::new(pool, true).await;
        let public_catalog: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert_eq!(public_catalog.len(), 107);
        sqlx::raw_sql(include_str!(
            "/var/tmp/0113_owned_first_ready_lease_exclusion.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
        let after_catalog: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
        )
        .fetch_all(pool)
        .await
        .unwrap();
        assert_eq!(
            public_catalog, after_catalog,
            "successor leaves genuine PUBLIC107 catalog checksums unchanged"
        );
        let root = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let owner =
            Arc::new(VolumeRootOwner::initialize(root.path(), "creator-fixture-host").unwrap());
        let mkfs = ["/usr/sbin/mkfs.ext4", "/sbin/mkfs.ext4"]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .expect("native e2fsprogs required");
        for (name, executable, counter) in [
            ("mkfs.ext4", mkfs.clone(), "formats"),
            ("e2fsck", mkfs.parent().unwrap().join("e2fsck"), "fscks"),
        ] {
            let path = tools.path().join(name);
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\nprintf 'call\\n' >> '{}'\nexec '{}' \"$@\"\n",
                    tools.path().join(counter).display(),
                    executable.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let actor = Arc::new(PostgresVolumeMetadataRepository::new(h.app.clone()));
        let worker = Arc::new(PostgresVolumeMetadataRepository::new(h.worker.clone()));
        let config = config(&root, &tools);
        let local = LocalVolumeStore::new(worker.clone(), config)
            .unwrap()
            .with_owned_metadata(OwnedVolumeMetadata::new(
                actor.clone(),
                worker.clone(),
                owner.clone(),
            ))
            .unwrap();

        Self {
            h,
            root,
            tools,
            owner,
            actor,
            worker,
            local,
        }
    }
    pub fn count(&self, name: &str) -> usize {
        std::fs::read_to_string(self.tools.path().join(name))
            .unwrap_or_default()
            .lines()
            .count()
    }
    pub async fn request(&self) -> (EffectClaim, BeginOwnedProvisioning) {
        self.h
            .preparation
            .prepare_execution(&self.h.actor, self.h.command, self.h.intent.id())
            .await
            .unwrap();
        let snapshot = self
            .h
            .repository
            .inspect(&self.h.actor, self.h.intent.id())
            .await
            .unwrap();
        let progress = &snapshot.resources[&fixtures::key("data")];
        let claim = self
            .h
            .repository
            .claim_resource_effect(
                &self.h.actor,
                ResourceEffect {
                    command: self.h.command,
                    deployment_id: self.h.intent.id(),
                    resource: fixtures::key("data"),
                    expected_deployment_version: snapshot.version,
                    expected_resource_version: progress.version,
                    action: ResourceAction::Create,
                    provenance: AttemptProvenance::new(
                        &self.h.actor,
                        DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                    )
                    .unwrap(),
                },
            )
            .await
            .unwrap();
        let mapped = self
            .h
            .repository
            .prepare_owned_volume_creation(&self.h.actor, &claim)
            .await
            .unwrap();
        let registration = OwnedVolumeRegistration::new(
            VolumeRegistration::new(
                mapped.volume_id,
                mapped.project_id,
                mapped.capacity_bytes,
                mapped.filesystem_uuid,
            )
            .unwrap(),
            mapped.creation,
        );
        let receipt = self
            .actor
            .register_owned(&self.h.actor, &registration)
            .await
            .unwrap();
        let expected = OwnedProvisioningExpectation::new(
            registration,
            self.owner.host().to_owned(),
            self.root.path().into(),
            self.owner.namespace_id(),
        )
        .unwrap();
        let request = BeginOwnedProvisioning {
            purpose: expected.purpose(receipt).unwrap(),
            operation_id: expected.first_operation_id(),
            expected_generation: 0,
        };
        (claim, request)
    }
}
fn config(root: &TempDir, tools: &TempDir) -> LocalVolumeConfig {
    LocalVolumeConfig {
        volume_root: root.path().into(),
        transient_runtime_roots: Vec::new(),
        host_id: "creator-fixture-host".into(),
        lease_duration: Duration::from_secs(30),
        mkfs_ext4: tools.path().join("mkfs.ext4"),
    }
}
pub async fn snapshot(pool: &PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('volumes',(SELECT jsonb_agg(to_jsonb(volume) ORDER BY id) FROM agent_instance_state_volumes volume),'operations',(SELECT jsonb_agg(to_jsonb(operation) ORDER BY id) FROM volume_owned_provisioning_operations operation),'observations',(SELECT jsonb_agg(to_jsonb(observation) ORDER BY id) FROM volume_owned_backing_observations observation),'verifications',(SELECT jsonb_agg(to_jsonb(proof) ORDER BY id) FROM recipe_effect_verifications proof),'transitions',(SELECT jsonb_agg(to_jsonb(transition) ORDER BY id) FROM recipe_execution_transitions transition),'leases',(SELECT jsonb_agg(to_jsonb(lease) ORDER BY id) FROM agent_instance_volume_leases lease),'resources',(SELECT jsonb_agg(to_jsonb(resource) ORDER BY deployment_id,resource_name) FROM recipe_deployment_resources resource),'deployments',(SELECT jsonb_agg(to_jsonb(deployment) ORDER BY id) FROM recipe_deployments deployment),'purposes',(SELECT jsonb_agg(to_jsonb(purpose) ORDER BY volume_id) FROM volume_owned_backing_purposes purpose),'preparations',(SELECT jsonb_agg(to_jsonb(prepared) ORDER BY deployment_id) FROM recipe_deployment_execution_preparations prepared))")
        .fetch_one(pool).await.unwrap()
}

pub async fn finish_held(
    f: &Fixture,
    claim: recipe_application::EffectClaim,
    custody: volume_local::OwnedReadyCustody,
) -> recipe_application::ResourceProgress {
    // A controlled owning completion path, not proof of the App creator driver.
    let preparation = f.h.preparation.clone();
    let recipe = f.h.repository.clone();
    let recorder = recipe_postgres::PostgresEffectVerificationRecorder::new(f.h.worker.clone());
    let identity = f.h.actor.clone();
    tokio::spawn(async move {
        let mut custody = custody;
        preparation
            .prepare_execution(&identity, claim.command, claim.deployment_id)
            .await
            .unwrap();
        custody.revalidate().unwrap();
        recorder
            .record(
                &claim,
                claim.provenance.attempt_id,
                recipe_postgres::VerifiedOutcome::Applied,
            )
            .await
            .unwrap();
        custody.revalidate().unwrap();
        let progress = recipe
            .complete_resource_effect(
                &identity,
                &claim,
                recipe_application::EffectEvidence {
                    identity: claim.identity,
                    input_hash: claim.input_hash,
                    outcome: recipe_application::EffectOutcome::Applied,
                },
            )
            .await
            .unwrap();
        drop(custody); // No new fsck after the commit opens runtime admission.
        progress
    })
    .await
    .unwrap()
}
