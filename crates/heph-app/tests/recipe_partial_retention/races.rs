use super::{
    fixture::{self, BirthMode, Fixture},
    host,
    worker::{PausedActor, PausedPartialWorker},
};
use crate::fixtures;
use sqlx::PgPool;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use volume_trait::{VolumeError, VolumePartialRetentionRepository};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL plus native e2fsprogs"]
async fn cached_creator_before_flock_cannot_mutate_after_retention_commit(pool: PgPool) {
    let f = Fixture::new(&pool, BirthMode::AllocatedWithdrawal).await;
    let (manager, request) = f.remove().await;
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(f.seeded.consuming_project)
        .bind(f.identity.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let actor = Arc::new(PausedActor {
        actual: f.actor.clone(),
        admitted: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
        first: AtomicBool::new(true),
    });
    let store = Arc::new(fixture::local(
        f.worker.clone(),
        actor.clone(),
        f.worker.clone(),
        f.owner.clone(),
        &f.root,
        &f.tools,
    ));
    let identity = f.identity.clone();
    let birth = f.request.clone();
    let waiting = tokio::spawn(async move { store.provision_owned(&identity, &birth).await });
    tokio::time::timeout(Duration::from_secs(10), actor.admitted.notified())
        .await
        .unwrap();
    let before = host::tree(f.root.path());
    f.coordinator()
        .retain(&manager, &request, &f.claim)
        .await
        .unwrap();
    actor.resume.notify_one();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(10), waiting)
            .await
            .unwrap()
            .unwrap(),
        Err(VolumeError::InvalidState(
            "owned birth is permanently retired"
        ))
    ));
    assert_eq!(host::tree(f.root.path()), before);
    assert_eq!(f.format_count(), 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL plus native e2fsprogs"]
async fn newer_actual_custody_receipt_invalidates_old_head_without_birth_mutation(pool: PgPool) {
    let f = Fixture::new(&pool, BirthMode::AllocatedWithdrawal).await;
    let (manager, request) = f.remove().await;
    let mut first = f
        .store
        .observe_owned_partial(f.worker.as_ref(), &f.claim)
        .await
        .unwrap();
    let old = f
        .worker
        .record_partial_retention_observation(first.context(), first.observation())
        .await
        .unwrap();
    first.revalidate().unwrap();
    drop(first);
    let mut second = f
        .store
        .observe_owned_partial(f.worker.as_ref(), &f.claim)
        .await
        .unwrap();
    let fresh = f
        .worker
        .record_partial_retention_observation(second.context(), second.observation())
        .await
        .unwrap();
    assert_eq!(
        fresh.observation.fields().observation_version,
        old.observation.fields().observation_version + 1
    );
    let before = host::tree(f.root.path());
    assert!(
        f.recipe
            .retain_partial_creation(&manager, &request, old.id.as_uuid())
            .await
            .is_err()
    );
    second.revalidate().unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_birth_retirements")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(host::tree(f.root.path()), before);
    assert_eq!(
        f.recipe
            .inspect(&manager, f.intent.id())
            .await
            .unwrap()
            .resources[&fixtures::key("data")]
            .active_attempt,
        Some(f.target.provenance.attempt_id)
    );
    drop(second);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private112 PostgreSQL plus native e2fsprogs"]
async fn cancelled_caller_keeps_actual_flock_until_waiting_actor_transaction_commits(pool: PgPool) {
    let f = Fixture::new(&pool, BirthMode::AllocatedWithdrawal).await;
    let (manager, request) = f.remove().await;
    let before = host::tree(f.root.path());
    let worker = Arc::new(PausedPartialWorker {
        actual: f.worker.clone(),
        recorded: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
    });
    let coordinator = crate::coordinator::PartialRetentionCoordinator::new(
        f.recipe.clone(),
        worker.clone(),
        f.store.clone(),
    );
    let identity = manager.clone();
    let input = request.clone();
    let claim = f.claim.clone();
    let caller = tokio::spawn(async move { coordinator.retain(&identity, &input, &claim).await });
    tokio::time::timeout(Duration::from_secs(10), worker.recorded.notified())
        .await
        .unwrap();
    let mut blocker = pool.begin().await.unwrap();
    let blocker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM projects WHERE id=$1 FOR NO KEY UPDATE")
        .bind(f.seeded.consuming_project)
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    worker.resume.notify_one();
    await_actor_wait(&pool, blocker_pid).await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    let file = std::fs::File::open(f.root.path().join(format!(
        ".{}.lock",
        f.request.purpose.receipt().intent.registration().id()
    )))
    .unwrap();
    assert!(
        rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).is_err()
    );
    blocker.commit().await.unwrap();
    for _ in 0..200 {
        let done: bool = sqlx::query_scalar("SELECT NOT owned_birth_is_live($1)")
            .bind(
                f.request
                    .purpose
                    .receipt()
                    .intent
                    .registration()
                    .id()
                    .as_uuid(),
            )
            .fetch_one(&pool)
            .await
            .unwrap();
        if done {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let plan = f
        .recipe
        .partial_creation_retention_plan(&manager, &request)
        .await
        .unwrap();
    assert!(plan.completed.is_some());
    for _ in 0..200 {
        if rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive).is_ok() {
            drop(file);
            assert_eq!(host::tree(f.root.path()), before);
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("protected completion did not release custody after committed result");
}

async fn await_actor_wait(pool: &PgPool, blocker_pid: i32) {
    for _ in 0..200 {
        let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE usename=current_user AND wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)) AND query LIKE 'SELECT organization_id FROM projects WHERE id = $1 FOR NO KEY UPDATE%')").bind(blocker_pid).fetch_one(pool).await.unwrap();
        if waiting {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("actual actor transaction did not reach project-row wait");
}
