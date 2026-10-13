use super::{
    fixture::{BirthMode, Fixture},
    host,
};
use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use sqlx::{PgPool, Postgres, Transaction};
use volume_trait::VolumePartialRetentionRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL plus native e2fsprogs"]
async fn actual_fence_and_event_roll_back_if_recipe_progress_is_not_committed(pool: PgPool) {
    let f = Fixture::new(&pool, BirthMode::AllocatedWithdrawal).await;
    let (manager, request) = f.remove().await;
    let mut custody = f
        .store
        .observe_owned_partial(f.worker.as_ref(), &f.claim)
        .await
        .unwrap();
    let receipt = f
        .worker
        .record_partial_retention_observation(custody.context(), custody.observation())
        .await
        .unwrap();
    let before = host::tree(f.root.path());
    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_events WHERE scope_id=$1")
            .bind(f.seeded.consuming_project)
            .fetch_one(&pool)
            .await
            .unwrap();
    let actors = crate::fixtures::app_pool(&pool).await;
    let mut tx = begin_actor_transaction(&actors, &manager).await.unwrap();
    authorize(
        &mut tx,
        &manager,
        ObjectRef::new(ObjectType::Project, f.seeded.consuming_project),
    )
    .await;
    authorize(
        &mut tx,
        &manager,
        ObjectRef::new(
            ObjectType::StateVolume,
            receipt.observation.fields().volume_id,
        ),
    )
    .await;
    sqlx::query("SELECT lock_recipe_partial_retention_volume($1,$2)")
        .bind(receipt.observation.fields().volume_id)
        .bind(f.seeded.consuming_project)
        .execute(&mut *tx)
        .await
        .unwrap();
    let state:serde_json::Value=sqlx::query_scalar("SELECT recipe_effect_resource_state(resource) FROM recipe_deployment_resources resource WHERE deployment_id=$1 AND resource_name='data' FOR UPDATE")
        .bind(f.intent.id().as_uuid()).fetch_one(&mut *tx).await.unwrap();
    let mut after = state.clone();
    after["active_attempt"] = serde_json::Value::Null;
    after["removal"] = "retained".into();
    after["version"] = (request.expected_resource_version + 1).into();
    insert_transition(
        &mut tx,
        &f,
        &manager,
        &request,
        receipt.id.as_uuid(),
        state,
        after,
    )
    .await;
    let staged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM volume_owned_birth_retirements WHERE volume_id=$1",
    )
    .bind(receipt.observation.fields().volume_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(staged, 1, "real guard stages permanent fence");
    // Intentionally omit the recipe resource/deployment CAS writes. The real
    // deferred fence and execution commit guards must abort the whole transaction.
    assert!(tx.commit().await.is_err());
    let fences: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_birth_retirements")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(fences, 0);
    let after_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM application_events WHERE scope_id=$1")
            .bind(f.seeded.consuming_project)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(events, after_events);
    custody.revalidate().unwrap();
    assert_eq!(host::tree(f.root.path()), before);
    let plan = f
        .recipe
        .partial_creation_retention_plan(&manager, &request)
        .await
        .unwrap();
    assert!(plan.completed.is_none());
    drop(custody);
    actors.close().await;
}

async fn authorize(
    tx: &mut Transaction<'_, Postgres>,
    identity: &identity_domain::AuthenticatedIdentity,
    object: ObjectRef,
) {
    let decision = PostgresMelangeAuthorizer
        .check(
            tx,
            Subject::User(identity.user_id),
            Permission::CanManage,
            object,
        )
        .await
        .unwrap();
    assert!(decision.is_allowed());
    audit_decision(
        tx,
        identity.user_id,
        Permission::CanManage,
        object,
        decision,
        identity.request_id,
    )
    .await
    .unwrap();
}

async fn insert_transition(
    tx: &mut Transaction<'_, Postgres>,
    f: &Fixture,
    identity: &identity_domain::AuthenticatedIdentity,
    request: &recipe_application::RetainPartialCreation,
    receipt: uuid::Uuid,
    before: serde_json::Value,
    after: serde_json::Value,
) {
    let id = uuid::Uuid::new_v5(
        &request.operation_id.as_uuid(),
        b"creation_retained_partial",
    );
    sqlx::query("SELECT set_config('hephaestus.occurrence_id',$1,true)")
        .bind(id.to_string())
        .execute(&mut **tx)
        .await
        .unwrap();
    let(event,cursor,version):(uuid::Uuid,i64,i64)=sqlx::query_as("SELECT event_id,cursor,aggregate_version FROM append_application_event($1,'project',$2,'project',$2,'project.changed','updated',NULL,$3,NULL)")
        .bind(id).bind(f.seeded.consuming_project).bind(f.seeded.organization).fetch_one(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO recipe_execution_transitions(id,command_id,attempt_id,operation_id,deployment_id,project_id,resource_name,kind,fingerprint,actor_id,request_id,before_state,after_state,before_deployment_version,after_deployment_version,before_lifecycle,after_lifecycle,partial_retention_receipt_id,event_id,event_cursor,event_aggregate_version) VALUES($1,$2,$3,$4,$5,$6,'data','creation_retained_partial',sha256(convert_to('rollback-fixture','UTF8')),$7,$8,$9,$10,$11,$12,'removing','removing',$13,$14,$15,$16)")
        .bind(id).bind(request.command.id().as_uuid()).bind(f.target.provenance.attempt_id.as_uuid()).bind(request.operation_id.as_uuid()).bind(f.intent.id().as_uuid()).bind(f.seeded.consuming_project)
        .bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid()).bind(before).bind(after)
        .bind(i64::try_from(request.expected_deployment_version).unwrap()).bind(i64::try_from(request.expected_deployment_version+1).unwrap())
        .bind(receipt).bind(event).bind(cursor).bind(version).execute(&mut **tx).await.expect("actual partial transition/fence guard, no Ready/absence evidence");
}
