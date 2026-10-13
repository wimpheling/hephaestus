use forge_domain::{ReceiveId, RefUpdate};
use forge_postgres::PgForgeRepository;
use release_domain::{NetworkAccess, RuntimePolicy};
use release_service::InstanceExecutionConfiguration;
use run_domain::{FreshRunExecutionProfile, LegacyVmPlacementScope, StartRun};
use run_orchestrator::RunRepository;
use run_postgres::{
    PgQualifiedInvocationRoutingReader, PgRunRepository, QualifiedOrdinaryRoute,
    QualifiedOrdinaryRoutingDecision,
};
use sqlx::PgPool;
use tempfile::TempDir;
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;
use volume_domain::VolumeRootNamespaceId;

use super::{git, parents::Parents, schema};

pub struct Fixture {
    pub(super) worker: PgPool,
    pub(super) application: PgPool,
    pub(super) reader: PgQualifiedInvocationRoutingReader,
    pub(super) runs: PgRunRepository,
    pub(super) start: StartRun,
    pub(super) scope: LegacyVmPlacementScope,
    pub(super) producer: Uuid,
    pub(super) receive: Uuid,
    pub(super) repository: Uuid,
    pub(super) commit: String,
    _root: TempDir,
}

impl Fixture {
    pub(super) async fn new(pool: &PgPool) -> Self {
        schema::install(pool).await;
        let root = tempfile::tempdir().expect("temporary genuine Git fixture");
        let parents = Parents::new();
        let (storage, commit) = git::commit(root.path(), parents.repository.id).await;
        parents.seed(pool, &commit).await;
        let receive = ReceiveId::new();
        // The real API explicitly permits absent historical identity. No invented
        // actor, fake grant or disabled database guard supplies this producer.
        let result = PgForgeRepository::new(pool.clone(), storage)
            .accept_receive_as(
                &parents.repository,
                receive,
                "ordinary-routing-fixture",
                None,
                &[RefUpdate {
                    git_ref: parents.repository.default_branch.clone(),
                    old_commit: None,
                    new_commit: Some(commit.clone()),
                }],
            )
            .await
            .expect("actual accepted Git producer");
        assert_eq!(result.run_requests.len(), 1);
        assert!(result.build_requests.is_empty());
        let request = result
            .run_requests
            .into_iter()
            .next()
            .expect("unique produced request");
        let worker = schema::role_pool(pool, true).await;
        let application = schema::role_pool(pool, false).await;
        let scope = owner();
        let reader = PgQualifiedInvocationRoutingReader::new(
            worker.clone(),
            configuration(&scope, "test/v1"),
        );
        let runs = PgRunRepository::new(worker.clone())
            .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
            .with_legacy_vm_placement(scope.clone());
        Self {
            worker,
            application,
            reader,
            runs,
            start: request.command,
            scope,
            producer: request.id.as_uuid(),
            receive: receive.as_uuid(),
            repository: parents.repository.id.as_uuid(),
            commit: commit.as_str().into(),
            _root: root,
        }
    }

    pub(super) async fn birth(&self) {
        assert!(
            self.runs
                .create_run_with_legacy_placement(&self.start, &self.scope)
                .await
                .expect("actual protected Legacy birth")
                .created
        );
    }

    pub(super) async fn original(&self) -> Box<QualifiedOrdinaryRoute> {
        qualified(
            self.reader
                .inspect_ordinary_original(self.start.run_id)
                .await
                .expect("worker original snapshot"),
        )
    }

    pub(super) async fn close(&self) {
        self.application.close().await;
        self.worker.close().await;
    }
}

pub fn owner() -> LegacyVmPlacementScope {
    LegacyVmPlacementScope::new(
        VmProviderOwnerScope::new(Uuid::new_v4().to_string(), "ordinary-routing-host".into())
            .expect("owner metadata"),
    )
    .expect("Legacy owner scope")
}

pub fn configuration(
    scope: &LegacyVmPlacementScope,
    version: &str,
) -> InstanceExecutionConfiguration {
    InstanceExecutionConfiguration::new(
        scope.provider_scope().clone(),
        "/ordinary-routing-volumes".into(),
        VolumeRootNamespaceId::from_uuid(Uuid::new_v4()).expect("volume namespace"),
        RuntimePolicy {
            vcpus: 1,
            memory_mib: 512,
            network: NetworkAccess::Disabled,
        },
        version.into(),
    )
    .expect("checked comparison configuration")
}

pub fn qualified(decision: QualifiedOrdinaryRoutingDecision) -> Box<QualifiedOrdinaryRoute> {
    match decision {
        QualifiedOrdinaryRoutingDecision::Qualified(original) => original,
        other => panic!("expected actual qualified original, observed {other:?}"),
    }
}
