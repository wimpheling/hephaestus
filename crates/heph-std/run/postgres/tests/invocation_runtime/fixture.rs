//! Actual SQL and filesystem prerequisite, with metadata-only planned VM identity.

use super::{bootstrap::Bootstrap, owned_births::OwnedBirths, schema};
use authz_postgres::PostgresMelangeAuthorizer;
use release_domain::ReleaseCommandKey;
use release_postgres::{PostgresInstanceExecutionService, ReleaseService};
use release_service::{
    ActivateInstance, InstanceActivationId, InstanceExecutionConfiguration,
    InstanceExecutionService, InstanceInvocationId, InvokeInstance,
};
use run_domain::{FreshRunExecutionProfile, RunCleanupHostId, RunKind, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError, RunRepository};
use run_postgres::PgRunRepository;
use runtime_types::{CommandId, RunId};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
use vm_trait::{VmId, VmProviderOwnerScope};
use volume_postgres::PostgresVolumeMetadataRepository;

pub struct Fixture {
    pub pool: PgPool,
    pub birth: Bootstrap,
    pub physical: OwnedBirths,
    pub config: InstanceExecutionConfiguration,
    pub start: StartRun,
    pub scope: RunCleanupHostId,
    pub vm: VmId,
}
impl Fixture {
    pub async fn new(pool: &PgPool, count: usize) -> Self {
        Box::pin(Self::build(pool, count, false)).await
    }
    pub async fn with_sibling(pool: &PgPool) -> Self {
        Box::pin(Self::build(pool, 0, true)).await
    }
    async fn build(pool: &PgPool, count: usize, sibling: bool) -> Self {
        let birth = Bootstrap::new(pool, count, sibling).await;
        let physical = OwnedBirths::new(pool, &birth, count).await;
        let claim = birth.claim("sqlite").await;
        let returned = ReleaseService::new(birth.app.clone(), Arc::new(PostgresMelangeAuthorizer))
            .import_agent_with_volumes(&birth.actor, crate::prepared_import::command(&birth.input))
            .await
            .unwrap();
        assert_eq!(returned, birth.input.instance_id());
        birth.complete(&claim).await;
        let config = InstanceExecutionConfiguration::new(
            VmProviderOwnerScope::new("vm-owner-fixture".to_owned(), "fixture-host".to_owned())
                .unwrap(),
            physical.root.path().to_str().unwrap().to_owned(),
            physical.owner.namespace_id(),
            crate::source_fixture::policy(512),
            "platform/activation-v1".to_owned(),
        )
        .unwrap();
        schema::upgrade(pool, 114).await;
        let service = PostgresInstanceExecutionService::new(
            birth.app.clone(),
            Arc::new(PostgresMelangeAuthorizer),
            config.clone(),
        );
        let activation = ActivateInstance::new(
            ReleaseCommandKey::derive("lease-activation", &[returned.as_uuid().as_bytes()]),
            InstanceActivationId::from_uuid(Uuid::new_v4()).unwrap(),
            returned,
            birth.input.revision_id(),
            1,
        )
        .unwrap();
        service
            .activate_instance(&birth.actor, activation)
            .await
            .unwrap();
        schema::upgrade(pool, 115).await;
        let id = Uuid::new_v4();
        let invocation = InvokeInstance::new(
            ReleaseCommandKey::derive("lease-invocation", &[id.as_bytes()]),
            InstanceInvocationId::from_uuid(id).unwrap(),
            RunId::new(),
            CommandId::new(),
            returned,
            birth.input.revision_id(),
        )
        .unwrap();
        service
            .with_qualified_invocation_admission()
            .invoke_instance(&birth.actor, invocation)
            .await
            .unwrap();
        let start: StartRun = sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT payload FROM command_inbox WHERE command_id=$1",
        )
        .bind(invocation.start_command_id().as_uuid())
        .fetch_one(pool)
        .await
        .map(|value| serde_json::from_value(value).unwrap())
        .unwrap();
        assert_eq!(start.kind, RunKind::Invocation);
        assert!(start.attachment_id.is_none());
        schema::upgrade(pool, 119).await;
        birth.assert_historical_migrations(pool).await;
        let scope = RunCleanupHostId::new(
            config.provider_scope().namespace().to_owned(),
            config.provider_scope().host_id().to_owned(),
        )
        .unwrap();
        let vm = VmId(start.run_id.to_string());
        Self {
            pool: pool.clone(),
            birth,
            physical,
            config,
            start,
            scope,
            vm,
        }
    }
    pub fn repository(&self) -> PgRunRepository {
        PgRunRepository::new(self.birth.worker.clone())
            .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
            .with_vm_owner_scope(self.scope.clone())
            .with_qualified_invocation_configuration(self.config.clone())
            .with_qualified_invocation_lease_runtime()
    }
    pub fn volumes(&self) -> PostgresVolumeMetadataRepository {
        PostgresVolumeMetadataRepository::new(self.birth.worker.clone())
            .with_qualified_invocation_configuration(self.config.clone())
    }
    pub async fn consume(&self) -> Result<CreateRunResult, RepositoryError> {
        self.repository()
            .create_run_with_vm_plan(&self.start, &self.scope, &self.vm)
            .await
    }
    pub async fn rows(&self) -> serde_json::Value {
        sqlx::query_scalar("SELECT jsonb_build_object('run',(SELECT to_jsonb(r) FROM runs r WHERE id=$1),'leases',(SELECT coalesce(jsonb_agg(to_jsonb(l) ORDER BY volume_id),'[]') FROM agent_instance_volume_leases l WHERE run_id=$1),'volumes',(SELECT coalesce(jsonb_agg(to_jsonb(v) ORDER BY v.id),'[]') FROM agent_instance_state_volumes v JOIN agent_instance_revision_volume_bindings b ON b.volume_id=v.id WHERE b.instance_revision_id=$2),'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY id) FROM application_events e),'inbox',(SELECT jsonb_agg(to_jsonb(i) ORDER BY command_id) FROM command_inbox i),'outbox',(SELECT jsonb_agg(to_jsonb(o) ORDER BY id) FROM outbox o),'products',(SELECT jsonb_agg(to_jsonb(o) ORDER BY event_id) FROM product_event_outbox o))")
            .bind(self.start.run_id.as_uuid()).bind(self.birth.input.revision_id().as_uuid()).fetch_one(&self.pool).await.unwrap()
    }
}
