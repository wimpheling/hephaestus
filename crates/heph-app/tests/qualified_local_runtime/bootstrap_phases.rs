//! Both genuine producer variants advance the same stopped backend together.

use super::super::{publication::PublishedVariant, source_variants::VolumeVariant};
use super::{Bootstrap, InvocationBirth, OwnedBirths, StoppedPublicationBackend, plan};
use release_domain::ReleaseCommandKey;
use release_postgres::{PostgresInstanceExecutionService, ReleaseService};
use release_service::{
    ActivateInstance, InstanceActivationId, InstanceExecutionService, InstanceInvocationId,
    InvokeInstance,
};
use run_domain::{FreshRunExecutionProfile, RunCleanupHostId, RunKind, StartRun};
use run_orchestrator::RunRepository;
use run_postgres::PgRunRepository;
use runtime_types::{AgentInstanceId, CommandId, RunId};
use std::{ptr, sync::Arc};
use uuid::Uuid;
use vm_trait::VmId;

struct ImportedVariant {
    bootstrap: Arc<Bootstrap>,
    physical: OwnedBirths,
    instance: AgentInstanceId,
}

pub(super) async fn invoke_pair(stopped: [StoppedPublicationBackend; 2]) -> [InvocationBirth; 2] {
    require_shared_backend(&stopped);
    let [zero, two] = stopped;
    let zero = Arc::new(Bootstrap::new(plan(zero).await).await);
    let two = Arc::new(Bootstrap::new(plan(two).await).await);
    assert_eq!(zero.historical_migrations, two.historical_migrations);
    let physical = OwnedBirths::new(&zero).await;
    let zero = zero.import(physical).await;
    let physical = OwnedBirths::new(&two).await;
    let two = two.import(physical).await;
    let pool = zero.bootstrap.published.pool.clone();
    crate::schema::upgrade(&pool, 114).await;
    let zero_service = zero.bootstrap.activate(zero.instance).await;
    let two_service = two.bootstrap.activate(two.instance).await;
    crate::schema::upgrade(&pool, 115).await;
    let zero_start = zero.bootstrap.invoke(zero.instance, zero_service).await;
    let two_start = two.bootstrap.invoke(two.instance, two_service).await;
    crate::schema::upgrade(&pool, 119).await;
    crate::authority_schema::apply(&pool).await;
    [
        zero.bootstrap.consume(zero.physical, zero_start).await,
        two.bootstrap.consume(two.physical, two_start).await,
    ]
}

// Compare retained in-memory identities before plan, owner validation or IO.
// SQLx options references reside inside PoolInner; connect_options returns its
// retained Arc, corroborating the same actual pool rather than equal URL text.
fn require_shared_backend(stopped: &[StoppedPublicationBackend; 2]) {
    assert!(stopped[0].same_handoff(&stopped[1]));
    stopped[0]
        .validate_handoff()
        .expect("same actual post-proof backend");
    stopped[1]
        .validate_handoff()
        .expect("same actual post-proof backend");
    let zero = stopped[0].publication();
    let two = stopped[1].publication();
    assert_eq!(zero.variant, VolumeVariant::Zero);
    assert_eq!(two.variant, VolumeVariant::Two);
    assert!(Arc::ptr_eq(&zero.provider, &two.provider));
    assert!(Arc::ptr_eq(&zero.owner, &two.owner));
    assert_eq!(zero.project_id(), two.project_id());
    assert_eq!(zero.actor.user_id, two.actor.user_id);
    require_same_pool(zero, two);
    require_same_configuration(zero, two);
    require_distinct_publications(zero, two);
}

fn require_same_pool(zero: &PublishedVariant, two: &PublishedVariant) {
    assert!(ptr::eq(zero.pool.options(), two.pool.options()));
    let zero_options = zero.pool.connect_options();
    let two_options = two.pool.connect_options();
    assert!(Arc::ptr_eq(&zero_options, &two_options));
    assert!(!zero.pool.is_closed() && !two.pool.is_closed());
}

fn require_same_configuration(zero: &PublishedVariant, two: &PublishedVariant) {
    assert_eq!(
        zero.configuration.canonical_json(),
        two.configuration.canonical_json()
    );
    assert_eq!(zero.local.volume_root, two.local.volume_root);
    assert_eq!(
        zero.local.transient_runtime_roots,
        two.local.transient_runtime_roots
    );
    assert_eq!(zero.local.host_id, two.local.host_id);
    assert_eq!(zero.local.lease_duration, two.local.lease_duration);
    assert_eq!(zero.local.mkfs_ext4, two.local.mkfs_ext4);
    assert_eq!(zero.runtime.runtime_root, two.runtime.runtime_root);
    assert_eq!(
        zero.runtime.release_artifact_root,
        two.runtime.release_artifact_root
    );
}

fn require_distinct_publications(zero: &PublishedVariant, two: &PublishedVariant) {
    assert_ne!(zero.published.repository_id, two.published.repository_id);
    assert_ne!(
        zero.published.build_request_id,
        two.published.build_request_id
    );
    assert_ne!(zero.published.release_id, two.published.release_id);
    assert_ne!(
        zero.published.release_agent_id,
        two.published.release_agent_id
    );
}

impl Bootstrap {
    async fn import(self: Arc<Self>, physical: OwnedBirths) -> ImportedVariant {
        self.stopped
            .validate_handoff()
            .expect("retained sealed Invocation handoff");
        self.published.validate_identity();
        assert!(Arc::ptr_eq(&physical.owner, &self.published.owner));
        assert_eq!(physical.root, self.published.local.volume_root);
        let claim = self.claim("sqlite").await;
        let instance = ReleaseService::new(
            self.app.clone(),
            Arc::new(authz_postgres::PostgresMelangeAuthorizer),
        )
        .import_agent_with_volumes(&self.actor, crate::prepared_import::command(&self.input))
        .await
        .unwrap();
        assert_eq!(instance, self.input.instance_id());
        self.complete(&claim).await;
        ImportedVariant {
            bootstrap: self,
            physical,
            instance,
        }
    }

    async fn activate(&self, instance: AgentInstanceId) -> PostgresInstanceExecutionService {
        let service = PostgresInstanceExecutionService::new(
            self.app.clone(),
            Arc::new(authz_postgres::PostgresMelangeAuthorizer),
            self.published.configuration.clone(),
        );
        let activation = ActivateInstance::new(
            ReleaseCommandKey::derive(
                "qualified-local-activation",
                &[instance.as_uuid().as_bytes()],
            ),
            InstanceActivationId::from_uuid(Uuid::new_v4()).unwrap(),
            instance,
            self.input.revision_id(),
            1,
        )
        .unwrap();
        service
            .activate_instance(&self.actor, activation)
            .await
            .unwrap();
        service
    }

    async fn invoke(
        &self,
        instance: AgentInstanceId,
        service: PostgresInstanceExecutionService,
    ) -> StartRun {
        let id = Uuid::new_v4();
        let invocation = InvokeInstance::new(
            ReleaseCommandKey::derive("qualified-local-invocation", &[id.as_bytes()]),
            InstanceInvocationId::from_uuid(id).unwrap(),
            RunId::new(),
            CommandId::new(),
            instance,
            self.input.revision_id(),
        )
        .unwrap();
        service
            .with_qualified_invocation_admission()
            .invoke_instance(&self.actor, invocation)
            .await
            .unwrap();
        let start = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT payload FROM command_inbox WHERE command_id=$1",
        )
        .bind(invocation.start_command_id().as_uuid())
        .fetch_one(&self.published.pool)
        .await
        .map(|value| serde_json::from_value::<StartRun>(value).unwrap())
        .unwrap();
        assert_eq!(start.kind, RunKind::Invocation);
        assert!(start.attachment_id.is_none());
        start
    }

    async fn consume(self: Arc<Self>, physical: OwnedBirths, start: StartRun) -> InvocationBirth {
        let after = sqlx::query_as::<_, (i64, Vec<u8>)>(
            "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
        )
        .fetch_all(&self.published.pool)
        .await
        .unwrap();
        assert_eq!(after, self.historical_migrations);
        let scope = RunCleanupHostId::new(
            self.published
                .configuration
                .provider_scope()
                .namespace()
                .to_owned(),
            self.published
                .configuration
                .provider_scope()
                .host_id()
                .to_owned(),
        )
        .unwrap();
        let vm = VmId(start.run_id.to_string());
        let runs = Arc::new(
            PgRunRepository::new(self.worker.clone())
                .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
                .with_vm_owner_scope(scope.clone())
                .with_qualified_invocation_configuration(self.published.configuration.clone())
                .with_qualified_invocation_lease_runtime(),
        );
        runs.create_run_with_vm_plan(&start, &scope, &vm)
            .await
            .unwrap();
        InvocationBirth {
            bootstrap: self,
            physical,
            runs,
            start,
            scope,
            vm,
        }
    }
}
