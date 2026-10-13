//! Genuine application construction and producer custody for an isolated database.

use super::super::HephaestusApp;
use super::{config, git, parents};
use heph_forge::{CommitSha, ReceiveId, RefUpdate, Repository};
use identity_domain::AuthenticatedIdentity;
use run_domain::StartRun;
use sqlx::{ConnectOptions, PgPool, migrate::Migrator};
use std::borrow::Cow;
use tempfile::TempDir;

const MIGRATIONS: Migrator = sqlx::migrate!("../../migrations");

pub(super) struct Fixture {
    pub(super) app: HephaestusApp,
    pub(super) pool: PgPool,
    pub(super) repository: Repository,
    pub(super) actor: AuthenticatedIdentity,
    pub(super) instance: runtime_types::AgentInstanceId,
    commit: CommitSha,
    _root: TempDir,
}

impl Fixture {
    pub(super) async fn new(pool: &PgPool) -> Self {
        install(pool, 143).await;
        let nats = nats_url();
        let root = tempfile::tempdir().expect("isolated application roots");
        let config = config::app_config(database_url(pool), nats, root.path());
        assert!(config.managed_backend.is_none());
        let policy = config.runtime_policy.clone();
        let image = config
            .root_images
            .keys()
            .next()
            .expect("one fixture image")
            .clone();
        let app = super::super::build(config, None)
            .await
            .expect("actual ordinary public143 assembly");
        let roles: (String, String, String) = (
            sqlx::query_scalar("SELECT current_user::text")
                .fetch_one(&app.pool)
                .await
                .unwrap(),
            sqlx::query_scalar("SELECT current_user::text")
                .fetch_one(&app.worker_pool)
                .await
                .unwrap(),
            sqlx::query_scalar("SELECT current_user::text")
                .fetch_one(&app.application_pool)
                .await
                .unwrap(),
        );
        assert_eq!(
            roles,
            (
                "postgres".into(),
                "hephaestus_worker".into(),
                "hephaestus_app".into()
            )
        );
        let repository = parents::repository();
        let commit = git::commit(&app.storage, root.path(), repository.id).await;
        let (actor, instance) = parents::seed(
            &app.pool,
            &app.artifact_store,
            &repository,
            &commit,
            &policy,
            &image,
            root.path(),
        )
        .await;
        Self {
            app,
            pool: pool.clone(),
            repository,
            actor,
            instance,
            commit,
            _root: root,
        }
    }

    pub(super) async fn historical(pool: &PgPool) -> (Self, StartRun) {
        use heph_run::RunRepository;
        install(pool, 107).await;
        let root = tempfile::tempdir().expect("isolated historical fixture");
        let config = config::app_config(database_url(pool), nats_url(), root.path());
        let storage = std::sync::Arc::new(
            forge_storage::GitStorage::initialize(&config.repository_root)
                .await
                .unwrap(),
        );
        let artifacts = release_artifact_store::LocalArtifactStore::new(
            config.run_runtime.release_artifact_root.clone(),
        )
        .unwrap();
        let repository = parents::repository();
        let commit = git::commit(&storage, root.path(), repository.id).await;
        let image = config.root_images.keys().next().unwrap();
        let (actor, instance) = parents::seed(
            pool,
            &artifacts,
            &repository,
            &commit,
            &config.runtime_policy,
            image,
            root.path(),
        )
        .await;
        let forge = forge_postgres::PgForgeRepository::new(pool.clone(), storage).with_authorizer(
            std::sync::Arc::new(authz_postgres::PostgresMelangeAuthorizer),
        );
        let result = forge
            .accept_receive_as(
                &repository,
                ReceiveId::new(),
                "genuine-public107-original",
                Some(&actor),
                &[RefUpdate {
                    git_ref: repository.default_branch.clone(),
                    old_commit: None,
                    new_commit: Some(commit.clone()),
                }],
            )
            .await
            .expect("actual public107 Git producer");
        assert_eq!(result.run_requests.len(), 1);
        let start = result.run_requests.into_iter().next().unwrap().command;
        assert!(
            run_postgres::PgRunRepository::new(pool.clone())
                .create_run(&start)
                .await
                .expect("actual public107 birth")
                .created
        );
        install(pool, 143).await;
        let app = super::super::build(config, None)
            .await
            .expect("actual upgraded public143 assembly");
        (
            Self {
                app,
                pool: pool.clone(),
                repository,
                actor,
                instance,
                commit,
                _root: root,
            },
            start,
        )
    }

    pub(super) async fn accept_git(&self) -> StartRun {
        let result = self
            .app
            .forge
            .accept_receive_as(
                &self.repository,
                ReceiveId::new(),
                "ordinary-public139-current-manager",
                Some(&self.actor),
                &[RefUpdate {
                    git_ref: self.repository.default_branch.clone(),
                    old_commit: None,
                    new_commit: Some(self.commit.clone()),
                }],
            )
            .await
            .expect("actual authenticated Git Normal producer");
        assert!(result.build_requests.is_empty());
        assert_eq!(result.run_requests.len(), 1);
        result
            .run_requests
            .into_iter()
            .next()
            .expect("one genuine original")
            .command
    }

    pub(super) async fn close(self) {
        self.app
            .nats_client
            .drain()
            .await
            .expect("unstarted test connection drain");
        self.app.application_pool.close().await;
        self.app.worker_pool.close().await;
        self.app.service_log_pool.close().await;
        self.app.pool.close().await;
    }
}

pub(super) fn database_url(pool: &PgPool) -> String {
    pool.connect_options().to_url_lossy().to_string()
}

fn nats_url() -> String {
    let nats = std::env::var("HEPHAESTUS_NATS_TEST_URL")
        .expect("explicit loopback disposable NATS prerequisite");
    let url = url::Url::parse(&nats).expect("NATS URL");
    assert_eq!(url.scheme(), "nats");
    assert!(matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "::1")
    ));
    nats
}

pub(super) async fn install(pool: &PgPool, version: i64) {
    assert_eq!(
        MIGRATIONS.iter().map(|m| m.version).collect::<Vec<_>>(),
        (1..=143).collect::<Vec<_>>()
    );
    Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|m| m.version <= version)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
    .run(pool)
    .await
    .expect("ordinary sealed public migration journal");
}
