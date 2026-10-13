use crate::{birth, fixtures};
use hephaestus_app::recipe_owned_volume::{OwnedVolumeCreateError, PreparedOwnedVolumeCreator};
use recipe_application::{DeploymentExecutionProfile, ResourceProgress};
use recipe_postgres::{PostgresEffectVerificationRecorder, PostgresExecutionPreparationRepository};
use sqlx::PgPool;
use std::{ops::Deref, sync::Arc, time::Duration};
use uuid::Uuid;
use volume_local::LocalVolumeConfig;
use volume_trait::{
    OwnedProvisioningExpectation, OwnedVolumeRegistration, VolumeOwnedProvisioningRepository,
    VolumeRegistration,
};

pub struct Fixture {
    pool: PgPool,
    birth: birth::Fixture,
    pub creator: PreparedOwnedVolumeCreator,
}
impl Deref for Fixture {
    type Target = birth::Fixture;
    fn deref(&self) -> &Self::Target {
        &self.birth
    }
}
impl Fixture {
    pub async fn new(pool: &PgPool) -> Self {
        let birth = birth::Fixture::new(pool).await;
        let creator = configured(&birth, birth.h.preparation.clone(), birth.h.worker.clone());
        Self {
            pool: pool.clone(),
            birth,
            creator,
        }
    }
    pub fn bad_recorder(&self) -> PreparedOwnedVolumeCreator {
        configured(&self.birth, self.h.preparation.clone(), self.h.app.clone())
    }
    pub fn drifted(&self) -> PreparedOwnedVolumeCreator {
        configured(
            &self.birth,
            Arc::new(PostgresExecutionPreparationRepository::new(
                self.h.app.clone(),
                crate::instance_fixture::platform(1024, "platform/drift"),
                DeploymentExecutionProfile::RuntimeNamedV1,
            )),
            self.h.worker.clone(),
        )
    }
    pub async fn create(&self) -> Result<ResourceProgress, OwnedVolumeCreateError> {
        self.creator
            .create_prepared_volume(
                &self.h.actor,
                self.h.command,
                self.h.intent.id(),
                &fixtures::key("data"),
            )
            .await
    }
    pub async fn stored_context(&self) -> volume_trait::OwnedProvisioningContext {
        let wire: serde_json::Value = sqlx::query_scalar("SELECT creation_json FROM recipe_owned_volume_preparations WHERE deployment_id=$1 AND resource_name='data'")
            .bind(self.h.intent.id().as_uuid()).fetch_one(&self.pool).await.unwrap();
        let creation = volume_trait::VolumeCreationIdentity::try_from(
            serde_json::from_value::<volume_trait::VolumeCreationIdentityWire>(wire).unwrap(),
        )
        .unwrap();
        let (volume, capacity, filesystem):(Uuid,i64,Uuid) = sqlx::query_as("SELECT volume_id,capacity_bytes,filesystem_uuid FROM recipe_owned_volume_preparations WHERE deployment_id=$1 AND resource_name='data'")
            .bind(self.h.intent.id().as_uuid()).fetch_one(&self.pool).await.unwrap();
        let expected = OwnedProvisioningExpectation::new(
            OwnedVolumeRegistration::new(
                VolumeRegistration::new(
                    runtime_types::VolumeId::from_uuid(volume),
                    self.h.fixture.consuming_project,
                    u64::try_from(capacity).unwrap(),
                    filesystem,
                )
                .unwrap(),
                creation,
            ),
            self.owner.host().to_owned(),
            self.root.path().into(),
            self.owner.namespace_id(),
        )
        .unwrap();
        let volume_trait::OwnedProvisioningDiscovery::Original(context) = self
            .worker
            .discover_owned_provisioning(&expected)
            .await
            .unwrap()
        else {
            panic!("actual original context required");
        };
        *context
    }
}
fn configured(
    f: &birth::Fixture,
    preparation: Arc<PostgresExecutionPreparationRepository>,
    recorder_pool: PgPool,
) -> PreparedOwnedVolumeCreator {
    PreparedOwnedVolumeCreator::new(
        preparation,
        f.h.repository.clone(),
        f.actor.clone(),
        f.worker.clone(),
        Arc::new(PostgresEffectVerificationRecorder::new(recorder_pool)),
        LocalVolumeConfig {
            volume_root: f.root.path().into(),
            transient_runtime_roots: Vec::new(),
            host_id: f.owner.host().to_owned(),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: f.tools.path().join("mkfs.ext4"),
        },
        f.owner.clone(),
    )
    .unwrap()
}
pub use birth::snapshot;
pub async fn wait_insert(pool: &PgPool, blocker: i32, stage: &str) {
    tokio::time::timeout(Duration::from_secs(10),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity activity WHERE activity.datname=current_database() AND pg_blocking_pids(activity.pid) @> ARRAY[$1::integer] AND activity.query LIKE $2)")
                .bind(blocker).bind(stage).fetch_one(pool).await.unwrap();
            if waiting { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("actual worker INSERT reaches held PostgreSQL relation, not a scheduling assumption");
}
