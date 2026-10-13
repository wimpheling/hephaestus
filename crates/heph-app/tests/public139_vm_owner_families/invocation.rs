use authz_postgres::PostgresMelangeAuthorizer;
use release_domain::{InstanceRemovalId, ReleaseCommandKey};
use release_postgres::ReleaseService;
use release_service::RequestInstanceRemoval;
use run_domain::{CancelRun, RunState, VmOwnerHistory, VmOwnerHistoryConflict};
use run_orchestrator::RunRepository;
use run_postgres::{
    PgQualifiedInvocationRoutingReader, PgVmOwnerHistoryReader, QualifiedInvocationEvidence,
    QualifiedInvocationRoutingDecision,
};
use runtime_types::CommandId;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
use vm_trait::{VmId, VmProviderOwnerScope};

#[path = "birth.rs"]
mod birth;
#[path = "prepared.rs"]
mod prepared;
#[path = "queued_cancel.rs"]
mod queued_cancel;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable ordinary public139 PostgreSQL through DATABASE_URL and local Git"]
async fn queued_consumed_cancelled_invocation_preserves_original_owner_after_closure(pool: PgPool) {
    let original = crate::fixture::Fixture::new(&pool).await;
    let config = crate::fixture::configuration(&original.scope, "test/v1");
    let owner = config.provider_scope();
    let reader = PgVmOwnerHistoryReader::new(original.worker.clone());
    assert_eq!(
        reader.observe(owner).await.unwrap(),
        VmOwnerHistory::NoHistoryForConfiguredHost
    );
    let (actor, imported) = prepared::import(&pool, &original, &config).await;
    assert_eq!(
        reader.observe(owner).await.unwrap(),
        VmOwnerHistory::NoHistoryForConfiguredHost,
        "genuine import completion alone has not admitted a Run"
    );
    let (start, runs, scope) = birth::admit(&pool, &original, &actor, &imported, &config).await;
    let decision = PgQualifiedInvocationRoutingReader::new(original.worker.clone(), config.clone())
        .inspect_start(&start)
        .await
        .unwrap();
    let QualifiedInvocationRoutingDecision::Qualified(queued) = decision else {
        panic!("genuine queued Invocation original must qualify");
    };
    assert_eq!(queued.original_start(), &start);
    assert_eq!(
        queued.evidence(),
        QualifiedInvocationEvidence::QueuedIntent115
    );
    assert_eq!(queued.run_state(), RunState::Queued);
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    let immutable = original_receipt(&pool, start.run_id.as_uuid()).await;
    assert_eq!(consumptions(&pool, start.run_id.as_uuid()).await, 0);
    let consumed = runs
        .create_run_with_vm_plan(&start, &scope, &VmId(start.run_id.to_string()))
        .await
        .unwrap();
    assert!(
        !consumed.created,
        "Invocation admission already created the Run"
    );
    assert_eq!(consumed.run.id, start.run_id);
    assert_eq!(consumed.run.state, RunState::Queued);
    assert_eq!(
        runs.get(start.run_id).await.unwrap().state,
        RunState::Queued
    );
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    assert_eq!(
        consumptions(&pool, start.run_id.as_uuid()).await,
        1,
        "actual protected worker consumption, without provider IO"
    );
    assert!(
        runs.request_cancel(&CancelRun {
            command_id: CommandId::new(),
            run_id: start.run_id,
            reason: "owner history before provider IO".into(),
        })
        .await
        .unwrap()
    );
    runs.transition(start.run_id, RunState::Cancelled, None, None)
        .await
        .unwrap();
    assert_eq!(
        runs.get(start.run_id).await.unwrap().state,
        RunState::Cancelled
    );
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    close_instance(&pool, &original, &actor, imported.instance_id()).await;
    assert_eq!(
        immutable,
        original_receipt(&pool, start.run_id.as_uuid()).await
    );
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    let changed =
        VmProviderOwnerScope::new(Uuid::new_v4().to_string(), owner.host_id().into()).unwrap();
    assert_eq!(
        reader.observe(&changed).await.unwrap(),
        VmOwnerHistory::Conflict(VmOwnerHistoryConflict::NamespaceMismatch)
    );
    let error = sqlx::query("UPDATE runs SET vm_provider_namespace=$2 WHERE id=$1")
        .bind(start.run_id.as_uuid())
        .bind(changed.namespace())
        .execute(&original.worker)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("23000")
    );
    assert_eq!(reader.observe(owner).await.unwrap(), VmOwnerHistory::Exact);
    // No Running, successful result, cleanup receipt or physical absence is claimed.
    original.close().await;
}

async fn original_receipt(pool: &PgPool, run: Uuid) -> serde_json::Value {
    sqlx::query_scalar("SELECT to_jsonb(receipt) FROM agent_instance_execution_invocations receipt WHERE run_id=$1")
        .bind(run).fetch_one(pool).await.unwrap()
}

async fn consumptions(pool: &PgPool, run: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM run_invocation_consumptions WHERE run_id=$1")
        .bind(run)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn close_instance(
    pool: &PgPool,
    original: &crate::fixture::Fixture,
    actor: &identity_domain::AuthenticatedIdentity,
    instance: release_domain::AgentInstanceId,
) {
    let version: i64 = sqlx::query_scalar("SELECT version FROM agent_instances WHERE id=$1")
        .bind(instance.as_uuid())
        .fetch_one(pool)
        .await
        .unwrap();
    let removal = InstanceRemovalId::new();
    ReleaseService::new(
        original.application.clone(),
        Arc::new(PostgresMelangeAuthorizer),
    )
    .request_instance_removal(
        actor,
        RequestInstanceRemoval {
            command_key: ReleaseCommandKey::derive(
                "owner-history-close",
                &[removal.as_uuid().as_bytes()],
            ),
            removal_id: removal,
            instance_id: instance,
            expected_version: u64::try_from(version).unwrap(),
        },
    )
    .await
    .unwrap();
    let closed: bool = sqlx::query_scalar("SELECT NOT run_gate_open AND EXISTS(SELECT 1 FROM agent_instance_removal_requests WHERE instance_id=$1) FROM agent_instances WHERE id=$1")
        .bind(instance.as_uuid()).fetch_one(pool).await.unwrap();
    assert!(
        closed,
        "actual removal admission closes the mutable launch gate"
    );
}

#[path = "recovery.rs"]
mod recovery;
