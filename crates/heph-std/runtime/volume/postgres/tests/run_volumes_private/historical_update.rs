//! Historical update-only rows remain held without a guessed run identity.

use crate::{legacy, support::Fixture};
use authz_postgres::PostgresMelangeAuthorizer;
use release_postgres::{ReleaseService, ReleaseServiceError, UpdateDecision, UpdateHookResult};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

pub async fn seed(pool: &PgPool, seeded: &Fixture) -> (Uuid, Uuid) {
    let (instance, revision, volume) = legacy::consumer(pool, seeded).await;
    let candidate = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,false FROM agent_instance_revisions WHERE id=$2")
        .bind(candidate).bind(revision).execute(pool).await.expect("historical candidate revision");
    let update = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_updates(id,instance_id,expected_current_revision_id,candidate_revision_id,state,actor_id) VALUES($1,$2,$3,$4,'hook_running',$5)")
        .bind(update).bind(instance).bind(revision).bind(candidate).bind(seeded.maintainer.as_uuid()).execute(pool).await.expect("historical admitted update, run identity unknown");
    sqlx::query("UPDATE agent_instances SET state='updating',run_gate_open=false WHERE id=$1")
        .bind(instance)
        .execute(pool)
        .await
        .expect("historical update gate");
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,update_id,host_id,fencing_token,state,expires_at) VALUES(gen_random_uuid(),$1,$2,$3,'host',3,'active',now()+interval '1 hour')")
        .bind(volume).bind(instance).bind(update).execute(pool).await.expect("actual pre108 update-only evidence");
    sqlx::query(
        "UPDATE agent_instance_state_volumes SET lease_generation=3,state='attached' WHERE id=$1",
    )
    .bind(volume)
    .execute(pool)
    .await
    .expect("held historical resource");
    (update, volume)
}

pub async fn exercise(pool: &PgPool, (update, volume): (Uuid, Uuid)) {
    let service = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    let before: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(lease) FROM agent_instance_volume_leases lease WHERE update_id=$1",
    )
    .bind(update)
    .fetch_one(pool)
    .await
    .expect("immutable run-null history");
    let result = service
        .record_update_hook_result(
            update.to_string().parse().expect("typed update"),
            UpdateHookResult::Rejected(23),
        )
        .await;
    assert!(
        matches!(result, Err(ReleaseServiceError::InvalidUpdateLifecycle)),
        "unknown history cannot be released: {result:?}"
    );
    let after: serde_json::Value = sqlx::query_scalar(
        "SELECT to_jsonb(lease) FROM agent_instance_volume_leases lease WHERE update_id=$1",
    )
    .bind(update)
    .fetch_one(pool)
    .await
    .expect("held row after rollback");
    assert_eq!(before, after);
    assert_eq!(
        service
            .record_update_hook_result(
                update.to_string().parse().expect("typed update"),
                UpdateHookResult::Uncertain
            )
            .await
            .expect("uncertainty remains fenced and held"),
        UpdateDecision::CompatibilityUnknown
    );
    let held:(Option<Uuid>,String,Option<time::OffsetDateTime>,String,i64)=sqlx::query_as("SELECT lease.run_id,lease.state,lease.released_at,volume.state,volume.lease_generation FROM agent_instance_volume_leases lease JOIN agent_instance_state_volumes volume ON volume.id=lease.volume_id WHERE lease.update_id=$1 AND volume.id=$2")
        .bind(update).bind(volume).fetch_one(pool).await.expect("historical recovery evidence");
    assert_eq!(
        held,
        (
            None,
            "recovery_required".into(),
            None,
            "recovering".into(),
            3
        )
    );
    println!(
        "PRIVATE_SCALAR_HISTORY=1 run_null_update_only_held=1 rejection_callback_rollback=1 recovery_without_guessed_identity=1 no_ready_or_release=1"
    );
}
