use crate::fixture::Fixture;
use recipe_application::{DeploymentError, OwnedReadyRetentionRepository, RemovalProgress};
use recipe_postgres::PostgresOwnedReadyRetentionRepository;
use sqlx::PgPool;
use volume_postgres::PostgresReadyRetentionRepository;
use volume_trait::{VolumeError, VolumeReadyRetentionRepository};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn new_current_manager_remove_resumes_original_closure_without_borrowing_actor_or_cas(
    pool: PgPool,
) {
    let f = Fixture::new(&pool, false).await;
    let (old_identity, old_request) = f.remove(f.birth.h.actor.user_id).await;
    let closed = f
        .actor
        .begin_owned_ready_retention(&old_identity, &old_request)
        .await
        .unwrap();
    let original = f.original().await;
    f.manager(f.birth.h.fixture.outsider).await;
    f.withdraw_installer().await;
    let before = f.snapshot().await;
    assert!(matches!(
        f.actor
            .begin_owned_ready_retention(&old_identity, &old_request)
            .await,
        Err(DeploymentError::AuthorizationDenied | DeploymentError::Unavailable)
    ));
    assert_eq!(f.snapshot().await, before);
    let (new_identity, new_request) = f.remove(f.birth.h.fixture.outsider).await;
    assert_eq!(
        new_request.expected_deployment_version, old_request.expected_deployment_version,
        "new current-manager admission does not rewrite the existing Removing CAS"
    );
    let correlations:i64=sqlx::query_scalar("SELECT count(*) FROM recipe_owned_ready_remove_admissions WHERE command_id=$1 AND closure_id=$2")
        .bind(new_request.command.id().as_uuid()).bind(closed.closure_id).fetch_one(&pool).await.unwrap();
    assert_eq!(correlations, 1);
    assert_ne!(new_request.command, old_request.command);
    assert_ne!(new_request.operation_id, old_request.operation_id);
    let resumed = f
        .actor
        .begin_owned_ready_retention(&new_identity, &new_request)
        .await
        .unwrap();
    assert_eq!(resumed.closure_id, closed.closure_id);
    let progress = f
        .coordinator()
        .retain(&new_identity, &new_request, &f.ready.claim)
        .await
        .unwrap();
    assert_eq!(progress.removal, RemovalProgress::Retained);
    assert_eq!(f.original().await, original);
    let closure_actor: uuid::Uuid = sqlx::query_scalar(
        "SELECT actor_id FROM volume_owned_ready_retention_closures WHERE id=$1",
    )
    .bind(closed.closure_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let disposition_actor: uuid::Uuid = sqlx::query_scalar(
        "SELECT actor_id FROM recipe_execution_transitions WHERE kind='creation_retained_ready'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(closure_actor, old_identity.user_id.as_uuid());
    assert_eq!(disposition_actor, new_identity.user_id.as_uuid());
    drop(f);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn worker_role_and_exact_opaque_scope_are_required_without_metadata_writes(pool: PgPool) {
    let f = Fixture::new(&pool, false).await;
    let (identity, request) = f.remove(f.birth.h.actor.user_id).await;
    // Metadata scope comparison accepts an opaque label. It asserts no VM ownership.
    let actor = PostgresOwnedReadyRetentionRepository::new(
        f.birth.h.app.clone(),
        f.birth.h.preparation.as_ref().clone(),
        "actual-owner".into(),
        "actual-host".into(),
    )
    .unwrap();
    let plan = actor
        .begin_owned_ready_retention(&identity, &request)
        .await
        .unwrap();
    let worker = PostgresReadyRetentionRepository::new(
        f.birth.h.worker.clone(),
        "actual-owner".into(),
        "actual-host".into(),
    )
    .unwrap();
    let before = f.snapshot().await;
    let context = worker
        .ready_retention_context(&f.ready.claim, plan.closure_id)
        .await
        .unwrap();
    assert_eq!(
        context.closure.fields().vm_provider_namespace,
        "actual-owner"
    );
    let actor_role = PostgresReadyRetentionRepository::new(
        f.birth.h.app.clone(),
        "actual-owner".into(),
        "actual-host".into(),
    )
    .unwrap();
    assert!(matches!(
        actor_role
            .ready_retention_context(&f.ready.claim, plan.closure_id)
            .await,
        Err(VolumeError::PermissionDenied)
    ));
    assert!(
        f.worker
            .ready_retention_context(&f.ready.claim, plan.closure_id)
            .await
            .is_err(),
        "configuration drift is not accepted"
    );
    let mut row_lock = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE")
        .bind(context.closure.fields().volume_id.as_uuid())
        .fetch_one(&mut *row_lock)
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        worker.ready_retention_context(&f.ready.claim, plan.closure_id),
    )
    .await
    .expect("readonly worker checks must not require row locks")
    .unwrap();
    row_lock.rollback().await.unwrap();
    assert_eq!(f.snapshot().await, before);
    drop(f);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn changed_original_claim_or_borrowed_remove_never_establishes_a_closure(pool: PgPool) {
    let f = Fixture::new(&pool, false).await;
    let (identity, request) = f.remove(f.birth.h.actor.user_id).await;
    let before = f.snapshot().await;
    let scope = heph_runtime::VmProvider::owner_scope(f.provider.as_ref()).unwrap();
    let drift = PostgresOwnedReadyRetentionRepository::new(
        f.birth.h.app.clone(),
        recipe_postgres::PostgresExecutionPreparationRepository::new(
            f.birth.h.app.clone(),
            crate::instance_fixture::platform(512, "changed-policy/v1"),
            recipe_application::DeploymentExecutionProfile::RuntimeNamedV1,
        ),
        scope.namespace().to_owned(),
        scope.host_id().to_owned(),
    )
    .unwrap();
    assert!(
        drift
            .begin_owned_ready_retention(&identity, &request)
            .await
            .is_err()
    );
    let mut changed = request.clone();
    changed.target.generation += 1;
    assert!(
        f.actor
            .begin_owned_ready_retention(&identity, &changed)
            .await
            .is_err()
    );
    let foreign = crate::support::identity(f.birth.h.fixture.outsider);
    assert!(
        f.actor
            .begin_owned_ready_retention(&foreign, &request)
            .await
            .is_err()
    );
    assert_eq!(f.snapshot().await, before);
    assert_eq!(f.birth.count("fscks"), 1);
    drop(f);
}
