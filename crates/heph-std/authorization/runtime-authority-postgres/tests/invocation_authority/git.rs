//! Requirements are authored before publication; no protected history is edited.
use super::harness::{code, pg_time};
use crate::{bootstrap::Bootstrap, prepared_import, schema};
use authz_postgres::PostgresMelangeAuthorizer;
use capability_domain::RuntimeSessionId;
use release_domain::ReleaseCommandKey;
use release_postgres::{PostgresInstanceExecutionService, ReleaseService};
use release_service::{
    ActivateInstance, InstanceActivationId, InstanceExecutionConfiguration,
    InstanceExecutionService, InstanceInvocationId, InvokeInstance,
};
use run_domain::{RunState, StartRun};
use run_orchestrator::RunRepository;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use runtime_types::{CommandId, RunId};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;
use vm_trait::{VmId, VmProviderOwnerScope};
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn required_git_actual_catalog_is_refused_by_checked_planning_before_preparation(
    pool: PgPool,
) {
    crate::bootstrap::required_git_planning_denied(&pool).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn optional_git_original_producer_cannot_issue119_session_or_copy_authority(pool: PgPool) {
    let (birth, physical, config, start) = Box::pin(prepared_optional_git(&pool)).await;
    let repo = provision_optional_git(&pool, &birth, &config, &start).await;
    sqlx::raw_sql(
        &std::fs::read_to_string("/var/tmp/0119_qualified_invocation_authority.sql").unwrap(),
    )
    .execute(&pool)
    .await
    .unwrap();
    let runtime = PgRuntimeSessionRepository::new(birth.worker.clone())
        .with_qualified_invocation_configuration(config.clone());
    let run = repo.get(start.run_id).await.unwrap();
    assert!(
        runtime.resolve_snapshot(&run, "git-denied").await.is_err(),
        "optional means declared Git, never omission permission"
    );
    let now = pg_time();
    let error = sqlx::query(
        "SELECT write_invocation119_session($1,$2,'git-denied',$3,$3,1,$3,$4,$5,$6,$7)",
    )
    .bind(start.run_id.as_uuid())
    .bind(serde_json::to_vec(&config.canonical_json()).unwrap())
    .bind([0_u8; 32].as_slice())
    .bind(now)
    .bind(now + time::Duration::seconds(60))
    .bind(now.unix_timestamp_nanos().to_string())
    .bind(
        (now + time::Duration::seconds(60))
            .unix_timestamp_nanos()
            .to_string(),
    )
    .execute(&birth.worker)
    .await
    .unwrap_err();
    assert_eq!(
        code(&error).as_deref(),
        Some("P1192"),
        "writer rejects Git before trusting supplied hash: {error:?}"
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM runtime_authority_sessions WHERE run_id=$1")
            .bind(start.run_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    assert!(
        runtime_authority::RuntimeSessionRepository::find(
            &runtime,
            RuntimeSessionId::from_uuid(start.run_id.as_uuid())
        )
        .await
        .is_err()
    );
    physical.assert_unchanged();
}

async fn prepared_optional_git(
    pool: &PgPool,
) -> (
    Bootstrap,
    crate::owned_births::OwnedBirths,
    InstanceExecutionConfiguration,
    StartRun,
) {
    let birth = Box::pin(Bootstrap::with_git(pool, false)).await;
    let physical = crate::owned_births::OwnedBirths::new(pool, &birth, 0).await;
    let claim = birth.claim("sqlite").await;
    let returned = ReleaseService::new(birth.app.clone(), Arc::new(PostgresMelangeAuthorizer))
        .import_agent_with_volumes(&birth.actor, prepared_import::command(&birth.input))
        .await
        .unwrap();
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
    service
        .activate_instance(
            &birth.actor,
            ActivateInstance::new(
                ReleaseCommandKey::derive(
                    "git-refusal-activation",
                    &[returned.as_uuid().as_bytes()],
                ),
                InstanceActivationId::from_uuid(Uuid::new_v4()).unwrap(),
                returned,
                birth.input.revision_id(),
                1,
            )
            .unwrap(),
        )
        .await
        .unwrap();
    schema::upgrade(pool, 115).await;
    let key = Uuid::new_v4();
    let command = InvokeInstance::new(
        ReleaseCommandKey::derive("git-refusal-invocation", &[key.as_bytes()]),
        InstanceInvocationId::from_uuid(key).unwrap(),
        RunId::new(),
        CommandId::new(),
        returned,
        birth.input.revision_id(),
    )
    .unwrap();
    service
        .with_qualified_invocation_admission()
        .invoke_instance(&birth.actor, command)
        .await
        .unwrap();
    let start: StartRun = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT payload FROM command_inbox WHERE command_id=$1",
    )
    .bind(command.start_command_id().as_uuid())
    .fetch_one(pool)
    .await
    .map(|value| serde_json::from_value(value).unwrap())
    .unwrap();
    (birth, physical, config, start)
}

async fn provision_optional_git(
    pool: &PgPool,
    birth: &Bootstrap,
    config: &InstanceExecutionConfiguration,
    start: &StartRun,
) -> run_postgres::PgRunRepository {
    schema::upgrade(pool, 119).await;
    let scope = run_domain::RunCleanupHostId::new(
        config.provider_scope().namespace().to_owned(),
        config.provider_scope().host_id().to_owned(),
    )
    .unwrap();
    let repo = run_postgres::PgRunRepository::new(birth.worker.clone())
        .with_execution_profile(run_domain::FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(scope.clone())
        .with_qualified_invocation_configuration(config.clone())
        .with_qualified_invocation_lease_runtime();
    repo.create_run_with_vm_plan(start, &scope, &VmId(start.run_id.to_string()))
        .await
        .unwrap();
    repo.transition(start.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let volumes = volume_postgres::PostgresVolumeMetadataRepository::new(birth.worker.clone())
        .with_qualified_invocation_configuration(config.clone());
    let selections = volumes.load_run_selections(start.run_id).await.unwrap();
    let now = pg_time();
    volumes
        .acquire_run(
            &selections,
            scope.host_id(),
            now,
            now + time::Duration::hours(2),
        )
        .await
        .unwrap();
    repo.bind_resources(start.run_id, None, None, None, &start.run_id.to_string())
        .await
        .unwrap();
    repo.transition(start.run_id, RunState::Provisioning, None, None)
        .await
        .unwrap();
    repo
}
