//! Actual matching configured `RunPG`, worker `RuntimeSessionPG` and Local store.
use std::{path::PathBuf, sync::Arc, time::Duration};

use authz_postgres::PostgresMelangeAuthorizer;
use control_plane_postgres::launch::PgRunLaunchAuthorizer;
use hephaestus_app::{CanonicalRunLaunchAuthorizer, PgRunAuthorityManager};
use release_service::InstanceExecutionConfiguration;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use runtime_git_authority_postgres::PgRuntimeGitCredentialRepository;
use sqlx::PgPool;
use volume_local::{LocalVolumeConfig, LocalVolumeStore};

use super::fixture::Fixture;

pub fn volumes(birth: &crate::fixture::Fixture) -> Arc<LocalVolumeStore> {
    let metadata = Arc::new(birth.volumes());
    Arc::new(
        LocalVolumeStore::new(
            metadata.clone(),
            LocalVolumeConfig {
                volume_root: birth.physical.root.path().into(),
                transient_runtime_roots: Vec::new(),
                host_id: birth.scope.host_id().to_owned(),
                lease_duration: Duration::from_secs(7200),
                mkfs_ext4: PathBuf::from("/usr/sbin/mkfs.ext4"),
            },
        )
        .unwrap()
        .with_run_metadata(metadata),
    )
}

pub fn manager(
    f: &Fixture,
    pool: PgPool,
    config: InstanceExecutionConfiguration,
    ttl: Duration,
) -> PgRunAuthorityManager {
    PgRunAuthorityManager::new_qualified_invocation(
        pool.clone(),
        PgRuntimeGitCredentialRepository::new(pool),
        (f.handoff.path().into(), [73; 32]),
        ttl,
        config,
        Arc::new(f.birth.repository()),
        f.volumes.clone(),
    )
    .unwrap()
}

pub fn launcher(
    f: &Fixture,
    pool: PgPool,
    config: InstanceExecutionConfiguration,
) -> CanonicalRunLaunchAuthorizer {
    CanonicalRunLaunchAuthorizer::new_qualified_invocation(
        PgRuntimeSessionRepository::new(pool.clone()),
        Arc::new(PgRunLaunchAuthorizer::new(
            pool,
            Arc::new(PostgresMelangeAuthorizer),
        )),
        config,
        Arc::new(f.birth.repository()),
    )
}
