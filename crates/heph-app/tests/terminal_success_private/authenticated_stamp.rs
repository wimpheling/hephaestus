//! Actual actor admission, Drain, App closure107/118 and monotonic replay.
use super::{fixture::Fixture, observations};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentOperation,
    PreparedInstanceClosureRepository, PreparedInstanceClosureRequest,
    PreparedInstanceSafetyRepository, ResourceAction, ResourceEffect,
};
use release_postgres::PostgresPreparedInstanceSafetyRepository;
use serde_json::Value;
use uuid::Uuid;

async fn request(
    f: &Fixture,
) -> (
    PostgresPreparedInstanceSafetyRepository,
    PreparedInstanceClosureRequest,
) {
    let birth = &f.birth.birth;
    let actor = &birth.actor;
    let snapshot = birth
        .recipe
        .inspect(actor, birth.intent.id())
        .await
        .unwrap();
    let command = CommandIdentity::from_identity(actor, DeploymentOperation::Remove).unwrap();
    birth
        .recipe
        .admit_remove(
            actor,
            recipe_application::RemoveDeployment {
                command,
                deployment_id: birth.intent.id(),
                expected_version: snapshot.version,
            },
        )
        .await
        .unwrap();
    let service = PostgresPreparedInstanceSafetyRepository::new(
        birth.app.clone(),
        birth.worker.clone(),
        f.birth.config.provider_scope(),
        birth.input.source().platform().clone(),
    );
    let target = service
        .load_original_target(actor, command, &birth.intent, birth.input.resource())
        .await
        .unwrap();
    let snapshot = birth
        .recipe
        .inspect(actor, birth.intent.id())
        .await
        .unwrap();
    let drain = birth
        .recipe
        .claim_resource_effect(
            actor,
            ResourceEffect {
                command,
                deployment_id: birth.intent.id(),
                resource: birth.input.resource().clone(),
                expected_deployment_version: snapshot.version,
                expected_resource_version: snapshot.resources[birth.input.resource()].version,
                action: ResourceAction::Drain,
                provenance: AttemptProvenance::new(
                    actor,
                    DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                )
                .unwrap(),
            },
        )
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("SELECT version FROM agent_instances WHERE id=$1")
        .bind(birth.input.instance_id().as_uuid())
        .fetch_one(&f.birth.pool)
        .await
        .unwrap();
    let request = PreparedInstanceClosureRequest::new(
        birth.intent.clone(),
        target,
        drain,
        u64::try_from(version).unwrap(),
    )
    .unwrap();
    (service, request)
}
pub async fn close_and_replay(f: &Fixture) {
    let birth = &f.birth.birth;
    let actor = &birth.actor;
    let (service, request) = request(f).await;
    let before = observations::rows(f).await;
    let files = observations::files(f);
    let admission = service.close_for_remove(actor, &request).await.unwrap();
    assert!(!admission.replayed);
    let after = observations::rows(f).await;
    stamp_only(f, &before, &after);
    let cancel: run_domain::CancelRun=serde_json::from_value(sqlx::query_scalar::<_,Value>("SELECT payload FROM outbox WHERE aggregate_id=$1 AND event_type='run.cancel_requested.v1'").bind(f.birth.start.run_id.as_uuid()).fetch_one(&f.birth.pool).await.unwrap()).unwrap();
    assert_eq!(cancel.run_id, f.birth.start.run_id);
    let replay = service.close_for_remove(actor, &request).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.target, admission.target);
    assert_eq!(
        observations::rows(f).await,
        after,
        "authenticated monotonic replay adds no Cancel or stamp change"
    );
    for (query, expected_code) in [
        (
            sqlx::query("UPDATE runs SET cancel_requested_at=NULL WHERE id=$1"),
            "23000",
        ),
        (
            sqlx::query(
                "UPDATE runs SET cancel_requested_at=cancel_requested_at+interval '1 microsecond' WHERE id=$1",
            ),
            "23000",
        ),
        // Existing table UPDATE/RLS rights reach the terminal marker/App immutable tuple guard.
        (
            sqlx::query(
                "UPDATE runs SET state='cleaned_up',state_version=state_version+1 WHERE id=$1",
            ),
            "23000",
        ),
    ] {
        let mut tx = authz_postgres::begin_actor_transaction(&birth.app, actor)
            .await
            .unwrap();
        let error = query
            .bind(f.birth.start.run_id.as_uuid())
            .execute(&mut *tx)
            .await
            .unwrap_err();
        assert_eq!(
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some(expected_code),
            "the actual App terminal guard must deny this change"
        );
        tx.rollback().await.unwrap();
        assert_eq!(observations::rows(f).await, after);
        assert_eq!(observations::files(f), files);
    }
    assert_eq!(observations::files(f), files);
    f.birth.physical.assert_unchanged();
}
fn stamp_only(f: &Fixture, before: &Value, after: &Value) {
    let old = observations::run(before, f);
    let new = observations::run(after, f);
    assert!(old["cancel_requested_at"].is_null());
    assert!(new["cancel_requested_at"].is_string());
    assert!(["succeeded", "cleaning_up"].contains(&old["state"].as_str().unwrap()));
    let mut expected = old;
    for field in ["updated_at", "cancel_requested_at"] {
        expected[field] = new[field].clone();
    }
    assert_eq!(
        new, expected,
        "App closure may stamp only, with exact state/version/result"
    );
    for table in [
        "invocation119_terminal_transactions",
        "run_invocation_consumptions",
        "agent_instance_execution_invocations",
        "agent_instance_volume_leases",
        "agent_instance_state_volumes",
        "run_cleanup_targets",
        "run_cleanup_target_leases",
        "run_cleanup_receipts",
        "run_cleanup_completions",
    ] {
        assert_eq!(before[table], after[table], "App closure preserves {table}");
    }
}
