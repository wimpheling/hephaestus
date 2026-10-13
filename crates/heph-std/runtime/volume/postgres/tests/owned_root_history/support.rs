use std::path::Path;

use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingPurpose, VolumeOwnedProvisioningRepository,
    VolumeRootNamespaceId,
};

use crate::seals::Fixture;

pub fn namespace() -> VolumeRootNamespaceId {
    VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).unwrap()
}

pub async fn purpose(fixture: &Fixture, host: &str, root: &str, namespace: VolumeRootNamespaceId) {
    let original = crate::backing::request(fixture).await;
    let request = BeginOwnedProvisioning {
        purpose: OwnedBackingPurpose::new(
            original.purpose.receipt().clone(),
            host.into(),
            Path::new(root).into(),
            namespace,
            1,
        )
        .unwrap(),
        operation_id: original.operation_id,
        expected_generation: original.expected_generation,
    };
    fixture
        .actor
        .begin_owned_provisioning(&crate::seals::identity(fixture.owner), &request)
        .await
        .expect("real immutable first operation/purpose; no physical proof claimed");
}

pub async fn other_project(pool: &PgPool, first: &Fixture) -> Fixture {
    let project = Uuid::new_v4();
    sqlx::query("INSERT INTO projects(id,organization_id,name) SELECT $1,organization_id,'other-root-project' FROM projects WHERE id=$2")
        .bind(project).bind(first.project).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(project)
        .bind(first.owner.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    Fixture {
        pool: pool.clone(),
        actor: first.actor.clone(),
        worker: first.worker.clone(),
        owner: first.owner,
        successor: first.successor,
        project,
    }
}
