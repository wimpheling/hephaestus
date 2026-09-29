//! End-to-end isolated build scenario setup.

#[path = "scenario/phases.rs"]
mod phases;

use authz_postgres::PostgresMelangeAuthorizer;
use build_orchestrator::{BuildExecutor, BuildExecutorConfig};
use build_postgres::PgBuildRepository;
use release_artifact_store::LocalArtifactStore;
use release_domain::BuildRequestId;
use release_postgres::ReleaseService;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicUsize},
    },
    time::Duration,
};
use tempfile::TempDir;
use uuid::Uuid;
use vm_trait::{RootFilesystem, VmProvider};

use super::support::{OutputProvider, seed, source_repository};

pub struct Scenario {
    pub pool: PgPool,
    pub _temporary: TempDir,
    pub root: PathBuf,
    pub artifact_root: PathBuf,
    pub artifact_store: LocalArtifactStore,
    pub executor: BuildExecutor,
    pub build_id: BuildRequestId,
    pub repository_id: Uuid,
    pub provisions: Arc<AtomicUsize>,
    pub fail_next_provision: Arc<AtomicBool>,
    pub image_filesystems: Arc<RwLock<BTreeMap<String, RootFilesystem>>>,
}

impl Scenario {
    async fn initialize(database_url: &str) -> Self {
        let pool = PgPoolOptions::new()
            .max_connections(6)
            .connect(database_url)
            .await
            .expect("connect PostgreSQL");
        sqlx::migrate!("../../../../../migrations")
            .run(&pool)
            .await
            .expect("apply migrations");
        let temporary = tempfile::tempdir().expect("temporary root");
        let root = temporary.path().canonicalize().expect("canonical root");
        let repository_root = root.join("repositories");
        let workspace_root = root.join("builds");
        let artifact_root = root.join("release-artifacts");
        let root_image = root.join("root-image");
        for directory in [
            &repository_root,
            &workspace_root,
            &artifact_root,
            &root_image,
        ] {
            fs::create_dir(directory).expect("private root");
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .expect("private mode");
        }
        let (repository_id, commit) = source_repository(&root, &repository_root);
        let build_id = seed(&pool, repository_id, &commit).await;
        let provisions = Arc::new(AtomicUsize::new(0));
        let fail_next_provision = Arc::new(AtomicBool::new(false));
        let provider: Arc<dyn VmProvider> = Arc::new(OutputProvider {
            provisions: Arc::clone(&provisions),
            fail_next_provision: Arc::clone(&fail_next_provision),
        });
        let releases = Arc::new(ReleaseService::new(
            pool.clone(),
            Arc::new(PostgresMelangeAuthorizer),
        ));
        let artifact_store =
            LocalArtifactStore::new(artifact_root.clone()).expect("artifact store");
        let image_filesystems = Arc::new(RwLock::new(BTreeMap::new()));
        let executor = BuildExecutor::initialize(
            Arc::new(PgBuildRepository::new(pool.clone())),
            provider,
            artifact_store.clone(),
            releases,
            BuildExecutorConfig {
                workspace_root,
                repository_root,
                git_binary: fs::canonicalize("/usr/bin/git").expect("Git binary"),
                image_filesystems: Arc::clone(&image_filesystems),
                timeout: Duration::from_secs(10),
            },
        )
        .expect("build executor");
        Self {
            pool,
            _temporary: temporary,
            root,
            artifact_root,
            artifact_store,
            executor,
            build_id,
            repository_id,
            provisions,
            fail_next_provision,
            image_filesystems,
        }
    }
}

#[tokio::test]
#[serial]
async fn exact_guest_output_becomes_one_immutable_draft() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let scenario = Scenario::initialize(&database_url).await;
    phases::initial(&scenario).await;
    phases::recovery(&scenario).await;
    phases::failure_and_authorization(&scenario).await;
}
