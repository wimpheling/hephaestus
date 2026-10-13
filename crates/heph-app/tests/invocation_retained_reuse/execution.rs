use super::preparation::Reuse;
use authz_postgres::PostgresMelangeAuthorizer;
use heph_runtime::{VmId, VmProvider};
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, EffectEvidence, EffectOutcome, ResourceAction,
    ResourceEffect,
};
use recipe_postgres::{PostgresEffectVerificationRecorder, VerifiedOutcome};
use release_domain::ReleaseCommandKey;
use release_postgres::PostgresInstanceExecutionService;
use release_service::{
    ActivateInstance, InstanceActivationId, InstanceExecutionConfiguration,
    InstanceExecutionService, InstanceInvocationId, InvokeInstance,
};
use run_domain::{FreshRunExecutionProfile, RunCleanupHostId, StartRun};
use run_postgres::PgRunRepository;
use runtime_types::{CommandId, RunId};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

pub struct Execution {
    pub configuration: InstanceExecutionConfiguration,
    pub start: StartRun,
    pub scope: RunCleanupHostId,
    pub vm: VmId,
}
impl Execution {
    pub async fn new(f: &crate::retained_fixture::Fixture, reuse: &Reuse) -> Self {
        Box::pin(Self::build(f, reuse)).await
    }
    async fn build(f: &crate::retained_fixture::Fixture, reuse: &Reuse) -> Self {
        complete(f, reuse, &reuse.claim).await;
        let snapshot = f
            .birth
            .h
            .repository
            .inspect(&reuse.identity, reuse.intent.id())
            .await
            .unwrap();
        let key = crate::fixtures::key("sqlite");
        let claim = f
            .birth
            .h
            .repository
            .claim_resource_effect(
                &reuse.identity,
                ResourceEffect {
                    command: reuse.command,
                    deployment_id: reuse.intent.id(),
                    resource: key.clone(),
                    expected_deployment_version: snapshot.version,
                    expected_resource_version: snapshot.resources[&key].version,
                    action: ResourceAction::Create,
                    provenance: AttemptProvenance::new(
                        &reuse.identity,
                        DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                    )
                    .unwrap(),
                },
            )
            .await
            .unwrap();
        let imported = f
            .birth
            .h
            .release
            .import_agent_with_volumes(
                &reuse.identity,
                hephaestus_app::recipe_instance_import::to_import_command(&reuse.input),
            )
            .await
            .unwrap();
        assert_eq!(imported, reuse.input.instance_id());
        complete(f, reuse, &claim).await;
        upgrade(&f.pool, 114).await;
        let configuration = configuration(f);
        let service = PostgresInstanceExecutionService::new(
            f.birth.h.app.clone(),
            Arc::new(PostgresMelangeAuthorizer),
            configuration.clone(),
        );
        let activation = ActivateInstance::new(
            ReleaseCommandKey::derive("retained-activation", &[imported.as_uuid().as_bytes()]),
            InstanceActivationId::from_uuid(Uuid::new_v4()).unwrap(),
            imported,
            reuse.input.revision_id(),
            1,
        )
        .unwrap();
        service
            .activate_instance(&reuse.identity, activation)
            .await
            .unwrap();
        upgrade(&f.pool, 115).await;
        let id = Uuid::new_v4();
        let invocation = InvokeInstance::new(
            ReleaseCommandKey::derive("retained-invocation", &[id.as_bytes()]),
            InstanceInvocationId::from_uuid(id).unwrap(),
            RunId::new(),
            CommandId::new(),
            imported,
            reuse.input.revision_id(),
        )
        .unwrap();
        service
            .with_qualified_invocation_admission()
            .invoke_instance(&reuse.identity, invocation)
            .await
            .unwrap();
        upgrade(&f.pool, 119).await;
        let start = load_start(&f.pool, invocation.start_command_id()).await;
        let scope = RunCleanupHostId::new(
            configuration.provider_scope().namespace().to_owned(),
            configuration.provider_scope().host_id().to_owned(),
        )
        .unwrap();
        let vm = VmId(start.run_id.to_string());
        Self {
            configuration,
            start,
            scope,
            vm,
        }
    }
    pub fn repository(&self, f: &crate::retained_fixture::Fixture) -> PgRunRepository {
        PgRunRepository::new(f.birth.h.worker.clone())
            .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
            .with_vm_owner_scope(self.scope.clone())
            .with_qualified_invocation_configuration(self.configuration.clone())
            .with_qualified_invocation_lease_runtime()
    }
}
async fn complete(
    f: &crate::retained_fixture::Fixture,
    reuse: &Reuse,
    claim: &recipe_application::EffectClaim,
) {
    PostgresEffectVerificationRecorder::new(f.birth.h.worker.clone())
        .record(claim, claim.provenance.attempt_id, VerifiedOutcome::Applied)
        .await
        .unwrap();
    f.birth
        .h
        .repository
        .complete_resource_effect(
            &reuse.identity,
            claim,
            EffectEvidence {
                identity: claim.identity,
                input_hash: claim.input_hash,
                outcome: EffectOutcome::Applied,
            },
        )
        .await
        .unwrap();
}
// Existing116 fixture already applied the exact116 SQL, outside migration
// bookkeeping. Append only missing target-local successors without inventing
// migration version rows or rewriting the genuine107->113 catalog.
pub async fn upgrade(pool: &PgPool, version: i64) {
    let prior: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap();
    if version == 114 {
        sqlx::raw_sql(include_str!(
            "/var/tmp/0114_qualified_instance_activation.corrected.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
    } else if version == 115 {
        sqlx::raw_sql(include_str!(
            "/var/tmp/0115_qualified_instance_invocation.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
    } else if version == 119 {
        sqlx::raw_sql(include_str!(
            "/var/tmp/0117_qualified_invocation_consumption.nowait.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "/var/tmp/0119_qualified_invocation_leases.sql"
        ))
        .execute(pool)
        .await
        .unwrap();
    } else {
        panic!("unsupported fixture-only successor");
    }
    let after: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(
        after, prior,
        "no fabricated public/private migration ledger records"
    );
}

fn configuration(f: &crate::retained_fixture::Fixture) -> InstanceExecutionConfiguration {
    // Controlled provider scope on the actual Local host is metadata only.
    // No physical VM is created, inspected or destroyed by this test.
    let controlled = vm_fake::FakeProvider::new_owned("creator-fixture-host".to_owned()).unwrap();
    InstanceExecutionConfiguration::new(
        controlled.owner_scope().unwrap(),
        f.birth.root.path().to_str().unwrap().to_owned(),
        f.birth.owner.namespace_id(),
        crate::source_fixture::policy(512),
        "platform/import-v1".to_owned(),
    )
    .unwrap()
}

async fn load_start(pool: &PgPool, command: CommandId) -> StartRun {
    sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT payload FROM command_inbox WHERE command_id=$1",
    )
    .bind(command.as_uuid())
    .fetch_one(pool)
    .await
    .map(|value| serde_json::from_value(value).unwrap())
    .unwrap()
}
