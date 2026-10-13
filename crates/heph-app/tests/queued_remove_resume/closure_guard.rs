//! Genuine live ordinary Control plus app-owned closure; no guest is started.
use crate::{assertions, fixture::Fixture, instance, queued, queued_remove_resume::denial, volume};
use identity_domain::{AuthenticatedIdentity, RequestId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentOperation,
    PlatformPolicyObservation, PreparedInstanceClosureRepository, PreparedInstanceClosureRequest,
    PreparedInstanceSafetyRepository, RemoveDeployment, RemoveResumeConfiguration, ResourceAction,
    ResourceEffect,
};
use recipe_postgres::PostgresDeploymentRepository;
use release_domain::InstanceRemovalId;
use release_postgres::PostgresPreparedInstanceSafetyRepository;
use serde_json::Value;
use sqlx::{Postgres, Transaction};

pub async fn run(fixture: &Fixture) {
    let prepared = instance::prepare(fixture).await;
    let volume = volume::create(fixture, &prepared).await;
    instance::import(fixture, &prepared).await;
    let starts = queued::activate_and_invoke(fixture, &prepared).await;
    let before = assertions::Snapshot::capture(fixture, &prepared.instance, &starts, volume).await;
    before.require_nonempty();
    let ordinary = queued::cancel(
        fixture,
        &prepared.actor,
        starts[0].run_id,
        "live ordinary Control before actual closure",
    )
    .await;
    let physical = denial::Backing::capture(fixture, volume).await;
    let original_runs = runs(fixture).await;
    let (publisher, removal) = close(fixture, &prepared).await;
    let changed_runs = runs(fixture).await;
    assert_eq!(original_runs.as_array().unwrap().len(), 2);
    assert_eq!(changed_runs.as_array().unwrap().len(), 2);
    for (old, new) in original_runs
        .as_array()
        .unwrap()
        .iter()
        .zip(changed_runs.as_array().unwrap())
    {
        assert!(old["cancel_requested_at"].is_null());
        assert!(!new["cancel_requested_at"].is_null());
        assert_eq!(new["state"], "queued");
        assert_eq!(without_stamp(old), without_stamp(new));
    }
    let genuine: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM control_requests WHERE id=$1 AND state='completed'),
         (SELECT count(*) FROM command_inbox WHERE command_id=$1),
         (SELECT count(*) FROM run_invocation_consumptions),
         (SELECT count(*) FROM outbox WHERE aggregate_type='release'
          AND subject='heph.run.command.cancel.v1' AND event_type='run.cancel_requested.v1')",
    )
    .bind(ordinary.command_id.as_uuid())
    .fetch_one(&fixture.admin)
    .await
    .unwrap();
    assert_eq!(genuine, (1, 0, 0, 2));
    let grants: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM agent_instance_volume_mount_grants WHERE instance_id=$1),
         (SELECT count(*) FROM agent_instance_volume_mount_revocations revoked
          JOIN agent_instance_volume_mount_grants grant_row ON grant_row.id=revoked.grant_id
          WHERE grant_row.instance_id=$1)",
    )
    .bind(prepared.instance.instance_id().as_uuid())
    .fetch_one(&fixture.admin)
    .await
    .unwrap();
    assert_eq!(grants, (1, 1));
    // Typed service replay remains an actual no-write replay, not a late stamp.
    late_stamp_denied(fixture, &publisher, removal).await;
    front_identity_denied(fixture, &publisher, removal).await;
    physical.assert_preserved(fixture).await;
}

fn without_stamp(value: &Value) -> Value {
    let mut value = value.clone();
    let object = value.as_object_mut().unwrap();
    assert!(object.remove("cancel_requested_at").is_some());
    assert!(object.remove("updated_at").is_some());
    value
}

async fn runs(fixture: &Fixture) -> Value {
    sqlx::query_scalar("SELECT jsonb_agg(to_jsonb(row) ORDER BY id) FROM runs row")
        .fetch_one(&fixture.admin)
        .await
        .unwrap()
}

async fn close(
    fixture: &Fixture,
    prepared: &instance::Prepared,
) -> (AuthenticatedIdentity, InstanceRemovalId) {
    let mut publisher = prepared.actor.clone();
    publisher.request_id = RequestId::new();
    publisher.idempotency_id = RequestId::new();
    let command = CommandIdentity::from_identity(&publisher, DeploymentOperation::Remove).unwrap();
    let platform = PlatformPolicyObservation::new(
        fixture.config.platform_policy().clone(),
        fixture.config.platform_version().into(),
    )
    .unwrap();
    let config = RemoveResumeConfiguration::new(
        fixture.config.provider_scope().namespace().into(),
        fixture.config.provider_scope().host_id().into(),
        platform.clone(),
        recipe_application::DeploymentExecutionProfile::RuntimeNamedV1,
    )
    .unwrap();
    let repository = PostgresDeploymentRepository::new(fixture.original.application.clone())
        .with_remove_resume(config.clone());
    let safety = PostgresPreparedInstanceSafetyRepository::new(
        fixture.original.application.clone(),
        fixture.original.worker.clone(),
        fixture.config.provider_scope(),
        platform,
    )
    .with_remove_resume(config)
    .unwrap();
    let snapshot = repository
        .inspect(&publisher, prepared.deployment)
        .await
        .unwrap();
    let admission = repository
        .admit_remove(
            &publisher,
            RemoveDeployment {
                command,
                deployment_id: prepared.deployment,
                expected_version: snapshot.version,
            },
        )
        .await
        .unwrap();
    let resource = instance::key("agent");
    let target = safety
        .load_original_target(&publisher, command, &admission.snapshot.intent, &resource)
        .await
        .unwrap();
    let drain = repository
        .claim_resource_effect(
            &publisher,
            ResourceEffect {
                command,
                deployment_id: prepared.deployment,
                resource: resource.clone(),
                expected_deployment_version: admission.snapshot.version,
                expected_resource_version: admission.snapshot.resources[&resource].version,
                action: ResourceAction::Drain,
                provenance: AttemptProvenance::new(
                    &publisher,
                    DeploymentAttemptId::from_uuid(uuid::Uuid::new_v4()).unwrap(),
                )
                .unwrap(),
            },
        )
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("SELECT version FROM agent_instances WHERE id=$1")
        .bind(prepared.instance.instance_id().as_uuid())
        .fetch_one(&fixture.admin)
        .await
        .unwrap();
    let request = PreparedInstanceClosureRequest::new(
        admission.snapshot.intent,
        target,
        drain,
        u64::try_from(version).unwrap(),
    )
    .unwrap();
    let closed = safety.close_for_remove(&publisher, &request).await.unwrap();
    assert!(!closed.replayed);
    let before_replay = denial::inventory(&fixture.admin).await;
    assert!(
        safety
            .close_for_remove(&publisher, &request)
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(before_replay, denial::inventory(&fixture.admin).await);
    (publisher, closed.removal_id)
}

async fn actor(
    tx: &mut Transaction<'_, Postgres>,
    publisher: &AuthenticatedIdentity,
    missing_request: bool,
) {
    sqlx::query(
        "SELECT set_config('search_path','pg_catalog,public,pg_temp',true),
        set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.subject_type','user',true),
        set_config('hephaestus.request_id',$2,true),set_config('hephaestus.occurrence_id',$3,true)",
    )
    .bind(publisher.user_id.to_string())
    .bind(if missing_request {
        String::new()
    } else {
        publisher.request_id.to_string()
    })
    .bind(publisher.idempotency_id.to_string())
    .execute(&mut **tx)
    .await
    .unwrap();
}

async fn late_stamp_denied(
    fixture: &Fixture,
    publisher: &AuthenticatedIdentity,
    removal: InstanceRemovalId,
) {
    let before = denial::inventory(&fixture.admin).await;
    let mut tx = fixture.original.application.begin().await.unwrap();
    actor(&mut tx, publisher, false).await;
    // This existing granted owning front executes the actual protected UPDATE.
    // Reaching the next statement excludes a missing table UPDATE privilege.
    let updated: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT close_instance_launches($1)")
        .bind(removal.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .unwrap();
    assert_eq!(updated.len(), 2);
    let failure = sqlx::query("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert_eq!(
        failure
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    assert_eq!(before, denial::inventory(&fixture.admin).await);
}

async fn front_identity_denied(
    fixture: &Fixture,
    publisher: &AuthenticatedIdentity,
    removal: InstanceRemovalId,
) {
    let before = denial::inventory(&fixture.admin).await;
    let mut tx = fixture.original.application.begin().await.unwrap();
    actor(&mut tx, publisher, true).await;
    // This is an earlier owning-front identity denial, not deferred guard proof.
    let failure = sqlx::query_scalar::<_, uuid::Uuid>("SELECT close_instance_launches($1)")
        .bind(removal.as_uuid())
        .fetch_all(&mut *tx)
        .await
        .unwrap_err();
    assert_eq!(
        failure
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    assert_eq!(before, denial::inventory(&fixture.admin).await);
}
