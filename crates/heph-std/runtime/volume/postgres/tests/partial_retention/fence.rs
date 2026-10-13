use super::support::{Fixture, begin, identity, partial, setup};
use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use volume_trait::{
    OwnedPartialBirthPhase, OwnedPartialRetentionReceipt, VolumePartialRetentionRepository,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private112 schema"]
async fn standalone_birth_fence_cannot_commit_without_atomic_recipe_transition(pool: PgPool) {
    let f = setup(&pool).await;
    let claim = begin(&f).await;
    let ctx = f.worker.partial_retention_context(&claim).await.unwrap();
    let proof = partial(&ctx, OwnedPartialBirthPhase::ClaimedEmpty);
    let receipt = f
        .worker
        .record_partial_retention_observation(&ctx, &proof)
        .await
        .unwrap();
    let command = seed_admitted_command(&pool, &f, &receipt).await;
    let mut tx = pool.begin().await.unwrap();
    insert_unpaired_fence(&mut tx, &f, &receipt, command).await;
    // Even privileged fixture SQL cannot commit a fence without its exact
    // recipe transition/state/audits. No triggers or roles are disabled.
    assert!(tx.commit().await.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_birth_retirements")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let live: bool = sqlx::query_scalar("SELECT owned_birth_is_live($1)")
        .bind(proof.fields().volume_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(live);
    assert_eq!(
        f.worker
            .partial_retention_context(&claim)
            .await
            .unwrap()
            .head
            .version,
        1
    );
}

async fn seed_admitted_command(
    pool: &PgPool,
    f: &Fixture,
    receipt: &OwnedPartialRetentionReceipt,
) -> Uuid {
    // A minimal real, audited admission supplies valid immediate foreign keys.
    // The negative test never supplies an execution transition or claims host proof.
    let actors = crate::seals::role_pool(pool, false).await;
    let actor = identity(f.owner);
    let mut tx = begin_actor_transaction(&actors, &actor).await.unwrap();
    let decision = PostgresMelangeAuthorizer
        .check(
            &mut tx,
            Subject::User(actor.user_id),
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, f.project),
        )
        .await
        .unwrap();
    assert!(decision.is_allowed());
    audit_decision(
        &mut tx,
        actor.user_id,
        Permission::CanManage,
        ObjectRef::new(ObjectType::Project, f.project),
        decision,
        actor.request_id,
    )
    .await
    .unwrap();
    let deployment = Uuid::new_v4();
    let command = Uuid::new_v4();
    sqlx::query("SELECT set_config('hephaestus.occurrence_id',$1,true)")
        .bind(command.to_string())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO recipe_definitions(project_id,recipe_id,recipe_version,contract_version,canonical_toml,declaration_hash,created_by,first_request_id) VALUES($1,'fence-fixture','1',1,'contract_version = 1',sha256(convert_to('fence-fixture','UTF8')),$2,$3)")
        .bind(f.project).bind(f.owner.as_uuid()).bind(actor.request_id.as_uuid()).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO recipe_deployments(id,project_id,deployment_key,recipe_id,recipe_version,declaration_hash,resolved_json,resolved_hash,input_hash,ordinary_inputs,admission_evidence,resource_count,created_by,first_request_id) VALUES($1,$2,'fence-fixture','fence-fixture','1',sha256(convert_to('fence-fixture','UTF8')),convert_to('{}','UTF8'),sha256(convert_to('{}','UTF8')),sha256(convert_to('{}','UTF8')),'{}','{}',1,$3,$4)")
        .bind(deployment).bind(f.project).bind(f.owner.as_uuid()).bind(actor.request_id.as_uuid()).execute(&mut *tx).await.unwrap();
    sqlx::query("INSERT INTO recipe_deployment_resources(deployment_id,project_id,resource_name,resource_kind,resource_id,filesystem_uuid,ownership,removal_policy,plan_json,input_hash) VALUES($1,$2,'data','volume',$3,$4,'owned','retain','{}',sha256(convert_to('{}','UTF8')))")
        .bind(deployment).bind(f.project).bind(receipt.observation.fields().volume_id)
        .bind(receipt.claim.purpose.receipt().intent.registration().filesystem_uuid()).execute(&mut *tx).await.unwrap();
    append_command(&mut tx, f, &actor, deployment, command).await;
    tx.commit().await.unwrap();
    actors.close().await;
    command
}

async fn append_command(
    tx: &mut Transaction<'_, Postgres>,
    f: &Fixture,
    actor: &identity_domain::AuthenticatedIdentity,
    deployment: Uuid,
    command: Uuid,
) {
    let (event,cursor,version): (Uuid,i64,i64) = sqlx::query_as("SELECT event_id,cursor,aggregate_version FROM append_application_event($1,'project',$2,'project',$2,'project.changed','updated',NULL,(SELECT organization_id FROM projects WHERE id=$2),NULL)")
        .bind(command).bind(f.project).fetch_one(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO recipe_deployment_commands(id,deployment_id,project_id,actor_id,operation,idempotency_id,input_hash,first_request_id,receipt_lifecycle,receipt_version,event_id,event_cursor,event_aggregate_version) VALUES($1,$2,$3,$4,'install',$1,sha256(convert_to('{}','UTF8')),$5,'installing',0,$6,$7,$8)")
        .bind(command).bind(deployment).bind(f.project).bind(f.owner.as_uuid()).bind(actor.request_id.as_uuid()).bind(event).bind(cursor).bind(version).execute(&mut **tx).await.unwrap();
}

async fn insert_unpaired_fence(
    tx: &mut Transaction<'_, Postgres>,
    f: &Fixture,
    receipt: &OwnedPartialRetentionReceipt,
    command: Uuid,
) {
    sqlx::query("INSERT INTO volume_owned_birth_retirements(volume_id,receipt_id,transition_id,command_id,operation_id,actor_id,request_id,project_id,fingerprint,event_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,sha256(convert_to('unpaired','UTF8')),$9)")
        .bind(receipt.observation.fields().volume_id).bind(receipt.id.as_uuid()).bind(Uuid::new_v4()).bind(command)
        .bind(Uuid::new_v4()).bind(f.owner.as_uuid()).bind(Uuid::new_v4()).bind(f.project).bind(receipt.event_id)
        .execute(&mut **tx).await.unwrap();
}
