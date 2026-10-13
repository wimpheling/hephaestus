use recipe_application::{
    ActiveDeploymentEffectRepository, CommandIdentity, DeploymentError, DeploymentOperation,
    ResourceAction,
};
use sqlx::PgPool;

use crate::{
    claim_fixture, fixtures,
    harness::{Graph, Harness},
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn source_withdrawal_denies_install_read_before_returning_original_claim(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let claim = claim_fixture::claim(&h, "data", ResourceAction::Create).await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(!h.can_use(&h.actor).await);
    let before = claim_fixture::state(&pool).await;
    assert!(matches!(
        claim_fixture::load(&h, &h.actor, h.command, &claim.resource).await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(before, claim_fixture::state(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn inactive_creator_is_denied_even_with_existing_original_claim(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let claim = claim_fixture::claim(&h, "data", ResourceAction::Create).await;
    for status in ["suspended", "disabled"] {
        sqlx::query("UPDATE users SET status=$1 WHERE id=$2")
            .bind(status)
            .bind(h.actor.user_id.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        let before = claim_fixture::state(&pool).await;
        assert!(matches!(
            claim_fixture::load(&h, &h.actor, h.command, &claim.resource).await,
            Err(DeploymentError::AuthorizationDenied)
        ));
        assert_eq!(before, claim_fixture::state(&pool).await);
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn different_admitted_manager_reads_old_install_after_creator_and_source_withdrawal(
    pool: PgPool,
) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let claim = claim_fixture::claim(&h, "data", ResourceAction::Create).await;
    let manager = claim_fixture::manager(&h, &pool).await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET status='suspended' WHERE id=$1")
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(!h.can_use(&h.actor).await);
    let command = claim_fixture::remove(&h, &manager).await;
    let before = claim_fixture::state(&pool).await;
    let loaded = claim_fixture::load(
        &h,
        &claim_fixture::fresh(&manager),
        command,
        &claim.resource,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(loaded, claim);
    assert_ne!(loaded.command.actor_id(), manager.user_id);
    assert_eq!(before, claim_fixture::state(&pool).await);
    // Cleanup reads also ignore configured policy drift; they never resume Install.
    assert_eq!(
        h.configured(1024, "changed")
            .load_active_effect(&manager, command, h.intent.id(), &claim.resource)
            .await
            .unwrap(),
        Some(claim.clone())
    );
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.consuming_project)
        .bind(manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = claim_fixture::state(&pool).await;
    assert!(matches!(
        claim_fixture::load(&h, &manager, command, &claim.resource).await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(before, claim_fixture::state(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn borrowed_unadmitted_and_wrong_resource_inputs_do_not_disclose_or_mutate_claim(
    pool: PgPool,
) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let claim = claim_fixture::claim(&h, "data", ResourceAction::Create).await;
    let manager = claim_fixture::manager(&h, &pool).await;
    let before = claim_fixture::state(&pool).await;
    assert!(matches!(
        claim_fixture::load(&h, &manager, h.command, &claim.resource).await,
        Err(DeploymentError::InputConflict)
    ));
    let command = CommandIdentity::from_identity(&manager, DeploymentOperation::Remove).unwrap();
    assert!(matches!(
        claim_fixture::load(&h, &manager, command, &claim.resource).await,
        Err(DeploymentError::Unavailable)
    ));
    assert!(matches!(
        claim_fixture::load(&h, &h.actor, h.command, &fixtures::key("foreign")).await,
        Err(DeploymentError::Unavailable)
    ));
    let mut changed = claim_fixture::fresh(&h.actor);
    changed.idempotency_id = identity_domain::RequestId::new();
    let unadmitted =
        CommandIdentity::from_identity(&changed, DeploymentOperation::Install).unwrap();
    assert!(matches!(
        claim_fixture::load(&h, &changed, unadmitted, &claim.resource).await,
        Err(DeploymentError::Unavailable)
    ));
    assert_eq!(before, claim_fixture::state(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn bound_external_rights_withdrawal_denies_before_claim_or_none(pool: PgPool) {
    let h = Harness::new(&pool, Graph::External, 113).await;
    let claim = claim_fixture::claim(&h, "data", ResourceAction::VerifyExternal).await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.consuming_project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = authz_postgres::begin_actor_transaction(&h.app, &h.actor)
        .await
        .unwrap();
    let rights:(i32,i32,i32)=sqlx::query_as("SELECT check_permission('user',$1,'can_read','state_volume',$2),
        check_permission('user',$1,'can_attach','state_volume',$2),check_permission('user',$1,'can_grant_agent_capability','state_volume',$2)")
        .bind(h.actor.user_id.to_string()).bind(h.fixture.volume.to_string()).fetch_one(&mut *tx).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(rights, (0, 0, 0));
    let before = claim_fixture::state(&pool).await;
    assert!(matches!(
        claim_fixture::load(&h, &h.actor, h.command, &claim.resource).await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(before, claim_fixture::state(&pool).await);
}
