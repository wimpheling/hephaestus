use heph_run::{Run, RunAuthorityManager, RunKind, RunState};
use run_orchestrator::RunLaunchAuthorizer;
use sqlx::PgPool;
use uuid::Uuid;

use super::{fixture::Fixture, wrapper};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable PostgreSQL and explicit private profile loader"]
async fn update_uses_original_provision_admission_not_mutable_update_actor(pool: PgPool) {
    let mut f = Fixture::new(pool).await;
    let update = fresh_update(&f).await;
    let authorizer = wrapper(&f);
    assert!(
        authorizer.authorize_with_volumes(&update).await.is_err(),
        "unconsumed no-IO admission grants no authority"
    );
    consume(&f, &update).await;
    f.run = update;
    f.run.vm_id = Some(f.run.id.to_string());
    f.advance(RunState::Provisioning).await;
    authorizer.authorize_with_volumes(&f.run).await.unwrap();
    let original: Uuid =
        sqlx::query_scalar("SELECT actor_id FROM run_vm_plan_admissions WHERE run_id=$1")
            .bind(f.run.id.as_uuid())
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(original, f.actor.maintainer.as_uuid());
    sqlx::query("UPDATE agent_updates SET actor_id=$2 WHERE id=$1")
        .bind(f.actor.update)
        .bind(f.actor.outsider.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    authorizer.authorize_with_volumes(&f.run).await.unwrap();
    drop(f.manager.prepare(&f.run).await.unwrap());
    f.advance(RunState::Starting).await;
    f.advance(RunState::Running).await;
    authorizer.authorize_with_volumes(&f.run).await.unwrap();
    sqlx::query("UPDATE agent_updates SET actor_id=$2 WHERE id=$1")
        .bind(f.actor.update)
        .bind(f.actor.owner.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(f.actor.consuming_project)
        .bind(original)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(authorizer.authorize_with_volumes(&f.run).await.is_err());
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(f.actor.consuming_project)
        .bind(original)
        .execute(&f.pool)
        .await
        .unwrap();
    authorizer.authorize_with_volumes(&f.run).await.unwrap();
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(f.actor.project)
        .bind(original)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(authorizer.authorize_with_volumes(&f.run).await.is_err());
    drop(authorizer);
    f.worker.close().await;
}

async fn fresh_update(f: &Fixture) -> Run {
    let (revision,agent,release,requires_state):(Uuid,Uuid,Uuid,bool)=sqlx::query_as(
        "SELECT revision.id,agent.id,agent.release_id,agent.requires_state FROM agent_updates update
         JOIN agent_instance_revisions revision ON revision.id=update.candidate_revision_id
         JOIN release_agents agent ON agent.id=revision.release_agent_id WHERE update.id=$1")
        .bind(f.actor.update).fetch_one(&f.pool).await.unwrap();
    let mut run = f.run.clone();
    run.id = Uuid::new_v4().into();
    run.command_id = Uuid::new_v4().into();
    run.instance_revision_id = revision.into();
    run.release_agent_id = agent.into();
    run.release_id = release.into();
    run.requires_state = requires_state;
    run.attachment_id = None;
    run.kind = RunKind::Update;
    run.state = RunState::Queued;
    let request = Uuid::new_v4();
    let key = [55_u8; 32];
    sqlx::query("UPDATE agent_instances SET state='updating',run_gate_open=false WHERE id=$1")
        .bind(run.instance_id.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('hephaestus.actor_id',$1,true),set_config('hephaestus.request_id',$2,true)")
        .bind(f.actor.maintainer.to_string()).bind(request.to_string()).execute(&mut *tx).await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO runs(id,command_id,instance_id,instance_revision_id,release_id,release_agent_id,run_kind,requires_state,state,created_at,updated_at,execution_profile,vm_plan_admission_required) VALUES($1,$2,$3,$4,$5,$6,'update',$7,'queued',now(),now(),'owned_canonical',true)")
        .bind(run.id.as_uuid()).bind(run.command_id.as_uuid()).bind(run.instance_id.as_uuid())
        .bind(revision).bind(release).bind(agent).bind(requires_state).execute(&mut *tx).await.unwrap();
    sqlx::query("SET LOCAL ROLE NONE")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE agent_updates SET state='hook_running',hook_run_id=$2 WHERE id=$1")
        .bind(f.actor.update)
        .bind(run.id.as_uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO release_command_inbox(command_key,operation,aggregate_id,secondary_id,actor_id,request_id) VALUES($1,'begin_update_hook',$2,$3,$4,$5)")
        .bind(key.as_slice()).bind(f.actor.update).bind(run.id.as_uuid()).bind(f.actor.maintainer.as_uuid())
        .bind(request).execute(&mut *tx).await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO run_vm_plan_admissions(run_id,command_id,instance_id,instance_revision_id,update_id,producer_command_key,actor_id,request_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(run.id.as_uuid()).bind(run.command_id.as_uuid()).bind(run.instance_id.as_uuid()).bind(revision)
        .bind(f.actor.update).bind(key.as_slice()).bind(f.actor.maintainer.as_uuid()).bind(request)
        .execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
    run
}

async fn consume(f: &Fixture, run: &Run) {
    let mut tx = f.worker.begin().await.unwrap();
    sqlx::query("UPDATE run_vm_plan_admissions SET consumed_at=now(),consumption_kind='provision',vm_id=run_id::text,vm_provider_namespace='manager-fixture-provider',vm_host_id='manager-fixture-host' WHERE run_id=$1")
        .bind(run.id.as_uuid()).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE runs SET vm_id=id::text,vm_provider_namespace='manager-fixture-provider',vm_host_id='manager-fixture-host',vm_binding_kind='planned' WHERE id=$1")
        .bind(run.id.as_uuid()).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}
