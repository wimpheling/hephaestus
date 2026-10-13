use super::fixture::Fixture;
use identity_domain::RequestId;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentOperation, ExistingInstanceClosureRepository,
    ExistingInstanceClosureRequest,
};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 chain; database observation only"]
async fn fresh_manager_loads_original_after_source_loss_with_fresh_request_no_writes(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool)).await;
    let before = f.seed.snapshot().await;
    let original = f
        .service()
        .load_existing_instance_closure(&f.seed.manager, &f.request)
        .await
        .unwrap();
    assert_eq!(original, *f.request.expected());
    assert_eq!(original.removal_id, f.seed.request.removal_id());
    assert_eq!(original.command_key, f.seed.request.command_key());
    assert_ne!(original.actor, f.seed.manager.user_id);
    let mut fresh = f.seed.manager.clone();
    fresh.request_id = RequestId::from_uuid(Uuid::new_v4());
    assert_ne!(fresh.request_id, f.seed.manager.request_id);
    assert_eq!(
        f.service()
            .load_existing_instance_closure(&fresh, &f.request)
            .await
            .unwrap(),
        original
    );
    assert_eq!(f.seed.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 chain"]
async fn default_unsupported_and_foreign_config_never_load_or_modify(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool)).await;
    let before = f.seed.snapshot().await;
    assert!(matches!(
        f.seed
            .closure
            .original
            .service()
            .load_existing_instance_closure(&f.seed.manager, &f.request)
            .await,
        Err(DeploymentError::InvalidAction)
    ));
    assert!(
        f.seed
            .closure
            .original
            .service()
            .with_remove_resume(
                recipe_application::RemoveResumeConfiguration::new(
                    "wrong-owner".into(),
                    f.seed.config.host().into(),
                    f.seed.config.platform().clone(),
                    f.seed.config.profile()
                )
                .unwrap()
            )
            .is_err()
    );
    assert!(
        f.foreign()
            .load_existing_instance_closure(&f.seed.manager, &f.request)
            .await
            .is_err()
    );
    assert_eq!(f.seed.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 chain"]
async fn current_management_withdrawal_denies_without_old_actor_or_source_fallback(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool)).await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(f.request.intent().project_id().as_uuid())
        .bind(f.seed.manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = f.seed.snapshot().await;
    assert!(matches!(
        f.service()
            .load_existing_instance_closure(&f.seed.manager, &f.request)
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(f.seed.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 chain"]
async fn substituted_original_fingerprint_and_request_are_held(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool)).await;
    let before = f.seed.snapshot().await;
    for change_request in [false, true] {
        let mut original = f.request.expected().clone();
        if change_request {
            original.request = RequestId::from_uuid(Uuid::new_v4());
        } else {
            original.input_hash =
                release_domain::ContentHash::digest(b"wrong original fingerprint");
        }
        let wrong = ExistingInstanceClosureRequest::new(
            f.request.command(),
            f.request.intent().clone(),
            f.request.target().clone(),
            original,
        )
        .unwrap();
        assert!(matches!(
            f.service()
                .load_existing_instance_closure(&f.seed.manager, &wrong)
                .await,
            Err(DeploymentError::ReconciliationRequired)
        ));
    }
    assert_eq!(f.seed.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 chain"]
async fn worker_role_and_disabled_current_actor_cannot_read_original(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool)).await;
    let worker = f
        .seed
        .closure
        .original
        .configured(
            f.seed.closure.original.base.worker.clone(),
            f.seed.closure.original.base.worker.clone(),
        )
        .with_remove_resume(f.seed.config.clone())
        .unwrap();
    let before = f.seed.snapshot().await;
    let raw = sqlx::query("SELECT load_existing_instance_closure118(NULL::uuid,NULL::uuid,NULL::uuid,NULL::bytea,NULL::uuid,NULL::jsonb,NULL::jsonb,NULL::text,NULL::text,NULL::jsonb)")
        .execute(&f.seed.closure.original.base.worker).await.unwrap_err();
    assert_eq!(
        raw.as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    assert!(matches!(
        worker
            .load_existing_instance_closure(&f.seed.manager, &f.request)
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(f.seed.snapshot().await, before);
    sqlx::query("UPDATE users SET status='disabled' WHERE id=$1")
        .bind(f.seed.manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = f.seed.snapshot().await;
    assert!(matches!(
        f.service()
            .load_existing_instance_closure(&f.seed.manager, &f.request)
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(f.seed.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private118 chain"]
async fn unadmitted_new_command_does_not_reconstruct_an_old_authority(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool)).await;
    let before = f.seed.snapshot().await;
    let mut unadmitted = f.seed.manager.clone();
    unadmitted.idempotency_id = RequestId::from_uuid(Uuid::new_v4());
    let command = CommandIdentity::from_identity(&unadmitted, DeploymentOperation::Remove).unwrap();
    let request = ExistingInstanceClosureRequest::new(
        command,
        f.request.intent().clone(),
        f.request.target().clone(),
        f.request.expected().clone(),
    )
    .unwrap();
    assert!(
        f.service()
            .load_existing_instance_closure(&unadmitted, &request)
            .await
            .is_err()
    );
    assert_eq!(f.seed.snapshot().await, before);
}
