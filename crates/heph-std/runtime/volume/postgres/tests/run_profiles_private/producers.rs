use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::fixture::{self, Execution};
use crate::support::Fixture;

pub async fn git(tx: &mut Transaction<'_, Postgres>, e: &Execution) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO command_inbox(command_id,command_type,payload) VALUES($1,'start_run',$2)",
    )
    .bind(e.command)
    .bind(e.payload())
    .execute(&mut **tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO run_requests(id,repository_id,commit_sha,git_ref,run_id,command_id,trigger_command_id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,platform_policy_version,request_kind,requires_state) SELECT gen_random_uuid(),repository_id,$1,'refs/heads/main',$2,$3,$3,$4,$5,$6,$7,$8,'fixture/v1','instance_normal',$9 FROM agent_attachments WHERE id=$8")
        .bind("a".repeat(40)).bind(e.run).bind(e.command).bind(e.instance).bind(e.revision).bind(e.release).bind(e.agent).bind(e.attachment).bind(e.requires_state)
        .execute(&mut **tx).await?;
    Ok(())
}

pub async fn normal(
    pool: &PgPool,
    e: &Execution,
    profile: &str,
    scope: bool,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await.unwrap();
    git(&mut tx, e).await?;
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    fixture::insert(&mut tx, e, profile, scope, false).await?;
    tx.commit().await
}

pub async fn mailbox(tx: &mut Transaction<'_, Postgres>, e: &Execution, f: &Fixture) {
    let mailbox = Uuid::new_v4();
    let payload = Uuid::new_v4();
    let event = Uuid::new_v4();
    sqlx::query("INSERT INTO mailboxes(id,project_id,instance_id,state) VALUES($1,$2,$3,'active')")
        .bind(mailbox)
        .bind(f.consuming_project)
        .bind(e.instance)
        .execute(&mut **tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO mailbox_payloads(id,mailbox_id,project_id,encoded_body,encoded_length,decoded_length,integrity_hash) VALUES($1,$2,$3,$4,1,1,$5)")
        .bind(payload).bind(mailbox).bind(f.consuming_project).bind(b"x".as_slice()).bind([0_u8;32].as_slice()).execute(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO mailbox_events(id,mailbox_id,project_id,instance_id,body_id,producer_kind,producer_id,deduplication_scope,deduplication_key,method,route,received_at) VALUES($1,$2,$3,$4,$5,'gateway','profile-fixture','profile-fixture',$6,'POST','/fixture',now())")
        .bind(event).bind(mailbox).bind(f.consuming_project).bind(e.instance).bind(payload).bind(event.to_string()).execute(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO mailbox_delivery_attempts(id,event_id,mailbox_id,attempt_number,state,command_id,instance_id,instance_revision_id,run_id,target_ref,target_commit,state_access_outcome) VALUES(gen_random_uuid(),$1,$2,1,'leased',$3,$4,$5,$6,'refs/heads/main',$7,'no_state')")
        .bind(event).bind(mailbox).bind(e.command).bind(e.instance).bind(e.revision).bind(e.run).bind("a".repeat(40)).execute(&mut **tx).await.unwrap();
}

pub async fn update(
    pool: &PgPool,
    f: &Fixture,
    owned: bool,
) -> (Execution, Result<(), sqlx::Error>) {
    let mut e = Execution::normal(pool, f).await;
    e.kind = "update";
    e.attachment = None;
    let candidate: Uuid =
        sqlx::query_scalar("SELECT candidate_revision_id FROM agent_updates WHERE id=$1")
            .bind(f.update)
            .fetch_one(pool)
            .await
            .unwrap();
    e.revision = candidate;
    // Source-authority fixture facts, not a delegated runtime grant.
    sqlx::query(
        "INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
    )
    .bind(f.project)
    .bind(f.maintainer.as_uuid())
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("UPDATE agent_instances SET state='updating',run_gate_open=false WHERE id=$1")
        .bind(e.instance)
        .execute(pool)
        .await
        .unwrap();
    let request = Uuid::new_v4();
    let key = [9_u8; 32];
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.request_id',$2,true)")
        .bind(f.maintainer.to_string()).bind(request.to_string()).execute(&mut *tx).await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    let profile = if owned {
        "owned_canonical"
    } else {
        "legacy_scalar"
    };
    if let Err(error) = fixture::insert(&mut tx, &e, profile, false, owned).await {
        return (e, Err(error));
    }
    sqlx::query("SET LOCAL ROLE NONE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE agent_updates SET state='hook_running',hook_run_id=$2 WHERE id=$1")
        .bind(f.update)
        .bind(e.run)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO release_command_inbox(command_key,operation,aggregate_id,secondary_id,actor_id,request_id) VALUES($1,'begin_update_hook',$2,$3,$4,$5)")
        .bind(key.as_slice()).bind(f.update).bind(e.run).bind(f.maintainer.as_uuid()).bind(request).execute(&mut *tx).await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    if owned {
        sqlx::query("INSERT INTO run_vm_plan_admissions(run_id,command_id,instance_id,instance_revision_id,update_id,producer_command_key,actor_id,request_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(e.run).bind(e.command).bind(e.instance).bind(e.revision).bind(f.update).bind(key.as_slice()).bind(f.maintainer.as_uuid()).bind(request).execute(&mut *tx).await.unwrap();
    }
    (e, tx.commit().await)
}
