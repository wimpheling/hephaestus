//! Deterministic database lock proof for grant versus permanent close.

use crate::{ReleaseService, roles, support};
use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use identity_domain::AuthenticatedIdentity;
use release_service::{InstanceRemovalAdmission, RequestInstanceRemoval};
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct GrantSource {
    instance_revision_id: Uuid,
    release_agent_id: Uuid,
    volume_id: Uuid,
    access_mode: String,
    runtime_contract_hash: Vec<u8>,
}

pub async fn grant_then_close(
    pool: &PgPool,
    seed: &support::Fixture,
    service: &ReleaseService,
    actor: &AuthenticatedIdentity,
    command: RequestInstanceRemoval,
) -> InstanceRemovalAdmission {
    // The seed pool supplies immutable fixture values. The actor has withdrawn
    // source-release read access, so an actor INSERT SELECT would select no row.
    // The actual INSERT below still enforces actor RLS and live volume authority.
    let source: GrantSource=sqlx::query_as("SELECT binding.instance_revision_id,binding.release_agent_id,binding.volume_id,binding.access_mode,agent.runtime_contract_hash FROM agent_instance_revision_volume_bindings binding JOIN release_agents agent ON agent.id=binding.release_agent_id WHERE binding.instance_id=$1 AND binding.slot_key='state'")
        .bind(seed.instance).fetch_one(pool).await.expect("exact existing legacy declaration");
    let actors = roles::role_pool(pool, roles::Role::Actor).await;
    let mut grant_tx = begin_actor_transaction(&actors, actor).await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *grant_tx)
        .await
        .unwrap();
    let grant = Uuid::new_v4();
    // Normal actor RLS and request context prove the database boundary. Holding
    // this INSERT's transaction open proves the trigger itself owns the lock.
    let inserted=sqlx::query("INSERT INTO agent_instance_volume_mount_grants(id,instance_revision_id,instance_id,release_agent_id,slot_key,volume_id,access_mode,release_contract_hash,created_by,request_id,authorization_model_version) VALUES($1,$2,$3,$4,'state',$5,$6,$7,$8,$9,$10)")
        .bind(grant).bind(source.instance_revision_id).bind(seed.instance).bind(source.release_agent_id).bind(source.volume_id).bind(source.access_mode).bind(source.runtime_contract_hash).bind(actor.user_id.as_uuid()).bind(actor.request_id.as_uuid()).bind(AUTHORIZATION_MODEL_VERSION).execute(&mut *grant_tx).await.expect("explicit actor grant holds same instance lock");
    assert_eq!(
        inserted.rows_affected(),
        1,
        "new grant must actually be inserted"
    );
    let removal = service.request_instance_removal(actor, command);
    tokio::pin!(removal);
    tokio::select! {
        result=&mut removal => panic!("close must wait for uncommitted grant: {result:?}"),
        ()=wait_for_blocker(pool,pid) => {}
    }
    grant_tx.commit().await.unwrap();
    let admission = tokio::time::timeout(Duration::from_secs(5), removal)
        .await
        .expect("close resumes after grant commit")
        .expect("permanent admission");
    let revoked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_instance_volume_mount_revocations WHERE grant_id=$1)",
    )
    .bind(grant)
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(
        revoked,
        "close lists/revokes grant committed before its instance lock"
    );
    actors.close().await;
    admission
}

async fn wait_for_blocker(pool: &PgPool, pid: i32) {
    tokio::time::timeout(Duration::from_secs(5),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity activity WHERE $1=ANY(pg_blocking_pids(activity.pid)))").bind(pid).fetch_one(pool).await.unwrap();
            if waiting { return; }
            tokio::task::yield_now().await;
        }
    }).await.expect("removal actually waits on grant's instance lock");
}

pub async fn progress_contention_fails_fast(pool: &PgPool, seed: &support::Fixture) {
    let mut closer = pool.begin().await.unwrap();
    sqlx::query("SELECT 1 FROM agent_instances WHERE id=$1 FOR UPDATE")
        .bind(seed.instance)
        .execute(&mut *closer)
        .await
        .unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(2),
        sqlx::query("UPDATE runs SET state='provisioning' WHERE id=$1")
            .bind(seed.run)
            .execute(pool),
    )
    .await
    .expect("row-progress guard must not wait in inverted lock order")
    .expect_err("contention fails closed");
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("55P03")
    );
    closer.rollback().await.unwrap();
}
