use release_service::InstanceExecutionService;
use run_domain::{FreshRunExecutionProfile, RunCleanupHostId, RunKind, StartRun};
use run_orchestrator::{CreateRunResult, RepositoryError, RunRepository};
use run_postgres::PgRunRepository;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use vm_trait::VmId;

use crate::producer_fixture::Invocation;

pub struct Fixture {
    pub producer: Invocation,
    pub start: StartRun,
    pub scope: RunCleanupHostId,
    pub vm: VmId,
}
impl Fixture {
    pub async fn new(pool: &PgPool) -> Self {
        Box::pin(Self::build(pool, false)).await
    }
    pub async fn bound(pool: &PgPool) -> Self {
        Box::pin(Self::build(pool, true)).await
    }
    async fn build(pool: &PgPool, bound: bool) -> Self {
        let producer = if bound {
            let activation = crate::activation_fixture::Harness::bound(pool).await;
            activation
                .service()
                .activate_instance(&activation.actor, activation.command)
                .await
                .unwrap();
            Invocation {
                command: invocation_command(&activation),
                activation,
            }
        } else {
            Invocation::new(pool).await
        };
        upgrade(pool, 115).await;
        producer
            .service()
            .invoke_instance(&producer.activation.actor, producer.command)
            .await
            .unwrap();
        let start: StartRun =
            sqlx::query_scalar::<_, Value>("SELECT payload FROM command_inbox WHERE command_id=$1")
                .bind(producer.command.start_command_id().as_uuid())
                .fetch_one(pool)
                .await
                .map(|value| serde_json::from_value(value).unwrap())
                .unwrap();
        assert_eq!(start.kind, RunKind::Invocation);
        assert_eq!(start.run_id, producer.command.run_id());
        upgrade(pool, 117).await;
        let owner = producer.activation.config.provider_scope();
        let scope = RunCleanupHostId::new(owner.namespace().to_owned(), owner.host_id().to_owned())
            .unwrap();
        let vm = VmId(start.run_id.to_string());
        Self {
            producer,
            start,
            scope,
            vm,
        }
    }
    pub fn repository(&self) -> PgRunRepository {
        PgRunRepository::new(self.producer.activation.worker.clone())
            .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
            .with_vm_owner_scope(self.scope.clone())
            .with_qualified_invocation_configuration(self.producer.activation.config.clone())
    }
    pub async fn consume(&self) -> Result<CreateRunResult, RepositoryError> {
        self.repository()
            .create_run_with_vm_plan(&self.start, &self.scope, &self.vm)
            .await
    }
    pub async fn rows(&self) -> Value {
        let producer = self.producer.rows().await;
        let consumed: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(receipt) ORDER BY run_id),'[]'::jsonb) FROM run_invocation_consumptions receipt")
            .fetch_one(&self.producer.activation.pool).await.unwrap();
        serde_json::json!({"producer":producer,"consumed":consumed})
    }
    pub async fn write(&self, tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT * FROM consume_qualified_invocation117($1,$2,$3,$4,$5,$6)")
            .bind(self.start.run_id.as_uuid())
            .bind(serde_json::to_value(&self.start).unwrap())
            .bind(&self.producer.activation.config_wire)
            .bind(self.scope.provider_namespace())
            .bind(self.scope.host_id())
            .bind(&self.vm.0)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }
}
fn invocation_command(
    activation: &crate::activation_fixture::Harness,
) -> release_service::InvokeInstance {
    use release_domain::ReleaseCommandKey;
    use release_service::{InstanceInvocationId, InvokeInstance};
    use runtime_types::{CommandId, RunId};
    use uuid::Uuid;
    let id = Uuid::new_v4();
    InvokeInstance::new(
        ReleaseCommandKey::derive("consume_bound_fixture", &[id.as_bytes()]),
        InstanceInvocationId::from_uuid(id).unwrap(),
        RunId::new(),
        CommandId::new(),
        activation.command.instance_id(),
        activation.command.expected_revision_id(),
    )
    .unwrap()
}
async fn upgrade(pool: &PgPool, target: i64) {
    let before: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap();
    let mut migrations = crate::preparation_schema::checkpoint(113);
    let mut all = migrations.iter().cloned().collect::<Vec<_>>();
    for (version, paths) in [
        (
            114,
            vec![include_str!(
                "../../../../../../migrations/0119_qualified_instance_activation.sql"
            )],
        ),
        (
            115,
            vec![include_str!(
                "../../../../../../migrations/0120_qualified_instance_invocation.sql"
            )],
        ),
        (
            116,
            vec![include_str!(
                "../../../../../../migrations/0121_owned_ready_retention.sql"
            )],
        ),
        (
            117,
            vec![include_str!(
                "../../../../../../migrations/0122_qualified_invocation_consumption.sql"
            )],
        ),
    ] {
        if version <= target {
            let sql = paths
                .iter()
                .copied()
                .map(str::to_owned)
                .collect::<Vec<_>>()
                .join("\n");
            all.push(sqlx::migrate::Migration::new(
                version,
                "private_invocation_consumption".into(),
                sqlx::migrate::MigrationType::Simple,
                sql.into(),
                false,
            ));
        }
    }
    migrations.migrations = all.into();
    migrations.run(pool).await.unwrap();
    let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<$1 ORDER BY version",
    )
    .bind(before.last().unwrap().0 + 1)
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(after, before, "prior107/private-chain checksums stay exact");
}
