use super::fixture::Fixture;
use recipe_application::{
    DeploymentError, PreparedInstanceSafetyRepository, PreparedInstanceSafetyTarget,
};
use release_domain::ContentHash;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn substituted_hash_and_another_original_attempt_are_not_adopted(pool: PgPool) {
    let f = Fixture::new(&pool, true, true).await;
    let before = f.snapshot().await;
    let mut claim = f.target.original().clone();
    claim.input_hash = ContentHash::digest(b"substituted original");
    assert!(matches!(
        f.service().observe_safety(&f.changed(claim)).await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    let mut claim = f.target.original().clone();
    claim.provenance.attempt_id =
        recipe_application::DeploymentAttemptId::from_uuid(uuid::Uuid::new_v4()).unwrap();
    assert!(matches!(
        f.service().observe_safety(&f.changed(claim)).await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    assert_eq!(f.snapshot().await, before);
}
#[sqlx::test(migrations = false)]
#[ignore = "requires fresh private118 PostgreSQL"]
async fn foreign_configuration_is_held_even_for_completed_zero_run_birth(pool: PgPool) {
    let f = Fixture::new(&pool, true, true).await;
    let manager = f.manager().await;
    let command = f.admit_remove(&manager).await;
    f.close(&manager).await;
    let before = f.snapshot().await;
    let foreign = PreparedInstanceSafetyTarget::new(
        f.target.original().clone(),
        f.base.input.clone(),
        "foreign-owner".into(),
        "actual-fixture-host".into(),
    )
    .unwrap();
    assert!(matches!(
        f.service().observe_safety(&foreign).await,
        Err(DeploymentError::IntentMismatch)
    ));
    let original = f
        .service()
        .inspect_for_remove(&manager, command, &f.target)
        .await
        .unwrap();
    assert_eq!(
        original.inventory.consumers,
        vec![f.base.input.instance_id()]
    );
    assert_eq!(f.snapshot().await, before);
}
