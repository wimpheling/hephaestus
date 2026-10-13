use crate::{fixture::Fixture, fixtures};
use recipe_application::{OwnedReadyRetentionRepository, RemovalProgress};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use volume_trait::VolumeReadyRetentionRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn cancelled_caller_cannot_release_native_custody_during_detached_fact_and_actor_commit(
    pool: PgPool,
) {
    let f = Arc::new(Fixture::new(&pool, false).await);
    let (identity, request) = f.remove(f.birth.h.actor.user_id).await;
    let plan = f
        .actor
        .begin_owned_ready_retention(&identity, &request)
        .await
        .unwrap();
    let context = f
        .worker
        .ready_retention_context(&f.ready.claim, plan.closure_id)
        .await
        .unwrap();
    let mut blocker = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("LOCK TABLE volume_owned_ready_retention_facts IN SHARE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let mut actor_blocker = pool.begin().await.unwrap();
    let actor_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *actor_blocker)
        .await
        .unwrap();
    sqlx::query("LOCK TABLE recipe_owned_ready_retentions IN SHARE MODE")
        .execute(&mut *actor_blocker)
        .await
        .unwrap();
    let caller = {
        let f = f.clone();
        let identity = identity.clone();
        let request = request.clone();
        tokio::spawn(async move {
            f.coordinator()
                .retain(&identity, &request, &f.ready.claim)
                .await
        })
    };
    wait_insert(&pool, pid).await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(
        f.birth
            .local
            .observe_owned_ready_retention(f.worker.as_ref(), &context)
            .await
            .is_err(),
        "actual journal lock remains with detached commit, not cancelled caller"
    );
    let fscks = f.birth.count("fscks");
    blocker.commit().await.unwrap();
    wait_actor_insert(&pool, actor_pid).await;
    assert!(
        f.birth
            .local
            .observe_owned_ready_retention(f.worker.as_ref(), &context)
            .await
            .is_err(),
        "actual journal custody also survives the cancelled caller while final actor INSERT waits"
    );
    actor_blocker.commit().await.unwrap();
    let progress = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = f
                .birth
                .h
                .repository
                .inspect(&identity, f.target.deployment_id)
                .await
                .unwrap();
            let progress = &snapshot.resources[&fixtures::key("data")];
            if progress.removal == RemovalProgress::Retained {
                break progress.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(progress.active_attempt.is_none());
    assert_eq!(
        f.coordinator()
            .retain(&identity, &request, &f.ready.claim)
            .await
            .unwrap(),
        progress
    );
    assert_eq!(
        f.birth.count("fscks"),
        fscks,
        "commit/replay never starts another fsck"
    );
    assert_eq!(f.birth.count("formats"), 1);
    drop(f);
}
async fn wait_insert(pool: &PgPool, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(10),async{
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)) AND query LIKE 'INSERT INTO volume_owned_ready_retention_facts%')")
                .bind(blocker).fetch_one(pool).await.unwrap();
            if waiting {break;}
            tokio::task::yield_now().await;
        }
    }).await.expect("actual PostgreSQL blocking relation proves worker INSERT reached while custody is owned");
}

async fn wait_actor_insert(pool: &PgPool, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(10),async{
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1=ANY(pg_blocking_pids(pid)) AND query LIKE 'INSERT INTO recipe_owned_ready_retentions%')")
                .bind(blocker).fetch_one(pool).await.unwrap();
            if waiting {break;}
            tokio::task::yield_now().await;
        }
    }).await.expect("actual PostgreSQL blocking relation proves final actor INSERT reached under detached custody");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn current_manager_withdrawal_before_actor_admission_keeps_ready_closed_without_original_completion(
    pool: PgPool,
) {
    let f = Arc::new(Fixture::new(&pool, false).await);
    let (identity, request) = f.remove(f.birth.h.actor.user_id).await;
    let plan = f
        .actor
        .begin_owned_ready_retention(&identity, &request)
        .await
        .unwrap();
    let context = f
        .worker
        .ready_retention_context(&f.ready.claim, plan.closure_id)
        .await
        .unwrap();
    let original = f.original().await;
    let actor = Arc::new(BeforeActorAdmission {
        actual: f.actor.clone(),
        entered: tokio::sync::Semaphore::new(0),
        proceed: tokio::sync::Semaphore::new(0),
    });
    let caller = {
        let f = f.clone();
        let actor = actor.clone();
        tokio::spawn(async move {
            hephaestus_app::recipe::ready_retention::ReadyRetentionCoordinator::new(
                actor,
                f.worker.clone(),
                Arc::new(f.birth.local.clone()),
                f.provider.clone(),
            )
            .retain(&identity, &request, &f.ready.claim)
            .await
        })
    };
    // Controlled scheduling AFTER real worker commit and BEFORE real actor TX.
    // Actual blocked-INSERT cancellation/custody is the separate test above.
    tokio::time::timeout(Duration::from_secs(10), actor.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    let facts: i64 = sqlx::query_scalar("SELECT count(*) FROM volume_owned_ready_retention_facts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(facts, 1, "genuine worker physical fact already committed");
    assert!(
        f.birth
            .local
            .observe_owned_ready_retention(f.worker.as_ref(), &context)
            .await
            .is_err(),
        "detached owner keeps actual journal custody across the pending actor delegation"
    );
    sqlx::query("DELETE FROM project_maintainers WHERE user_id=$1")
        .bind(f.birth.h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    actor.proceed.add_permits(1);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(10), caller)
            .await
            .unwrap()
            .unwrap(),
        Err(
            hephaestus_app::recipe::ready_retention::ReadyRetentionError::Deployment(
                recipe_application::DeploymentError::AuthorizationDenied
            )
        )
    ));
    let state:(bool,i64,bool)=sqlx::query_as("SELECT ready_retention_closure_id IS NOT NULL,(SELECT count(*) FROM recipe_owned_ready_retentions),recipe_owned_first_birth_completed(id) FROM agent_instance_state_volumes WHERE id=$1")
        .bind(f.ready.claim.purpose.receipt().intent.registration().id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(state, (true, 0, false));
    assert_eq!(f.original().await, original);
    assert_eq!(f.birth.count("formats"), 1);
    drop(actor);
    drop(f);
}

struct BeforeActorAdmission {
    actual: Arc<recipe_postgres::PostgresOwnedReadyRetentionRepository>,
    entered: tokio::sync::Semaphore,
    proceed: tokio::sync::Semaphore,
}
#[async_trait::async_trait]
impl OwnedReadyRetentionRepository for BeforeActorAdmission {
    async fn begin_owned_ready_retention(
        &self,
        identity: &identity_domain::AuthenticatedIdentity,
        request: &recipe_application::RetainOwnedReady,
    ) -> Result<recipe_application::OwnedReadyRetentionPlan, recipe_application::DeploymentError>
    {
        self.actual
            .begin_owned_ready_retention(identity, request)
            .await
    }
    async fn retain_owned_ready(
        &self,
        identity: &identity_domain::AuthenticatedIdentity,
        request: &recipe_application::RetainOwnedReady,
        fact: uuid::Uuid,
    ) -> Result<recipe_application::ResourceProgress, recipe_application::DeploymentError> {
        self.entered.add_permits(1);
        self.proceed
            .acquire()
            .await
            .map_err(|_| recipe_application::DeploymentError::Repository)?
            .forget();
        self.actual
            .retain_owned_ready(identity, request, fact)
            .await
    }
}
