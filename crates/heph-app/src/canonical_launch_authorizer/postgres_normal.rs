use heph_run::{RunAuthorityManager, RunState};
use run_orchestrator::RunLaunchAuthorizer;
use sqlx::PgPool;

use super::{fixture::Fixture, wrapper};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable PostgreSQL and explicit private profile loader"]
async fn original_normal_caller_execute_source_and_active_user_are_live(pool: PgPool) {
    let mut f = Fixture::new(pool).await;
    let authorizer = wrapper(&f);
    authorizer.authorize_with_volumes(&f.run).await.unwrap();
    let evidence = f.evidence().await;
    assert!(
        evidence["session"].is_null(),
        "launch checks never issue authority"
    );
    assert!(f.envelopes().is_empty());
    deny_withdrawn_role(&f, &authorizer, f.actor.consuming_project).await;
    deny_withdrawn_role(&f, &authorizer, f.actor.project).await;
    sqlx::query("UPDATE users SET status='disabled' WHERE id=$1")
        .bind(f.actor.maintainer.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(authorizer.authorize_with_volumes(&f.run).await.is_err());
    sqlx::query("UPDATE users SET status='active' WHERE id=$1")
        .bind(f.actor.maintainer.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    authorizer.authorize_with_volumes(&f.run).await.unwrap();
    assert_eq!(f.evidence().await, evidence);
    f.advance(RunState::Provisioning).await;
    drop(f.manager.prepare(&f.run).await.unwrap());
    for phase in [
        RunState::Starting,
        RunState::Running,
        RunState::Succeeded,
        RunState::CleaningUp,
    ] {
        f.advance(phase).await;
        authorizer.authorize_with_volumes(&f.run).await.unwrap();
    }
    let recorded = f.evidence().await;
    let envelope = f.envelopes();
    authorizer.authorize_with_volumes(&f.run).await.unwrap();
    assert_eq!(f.evidence().await, recorded);
    assert_eq!(f.envelopes(), envelope);
    f.manager.revoke_after_guest(f.run.id).await.unwrap();
    let revoked = f.evidence().await;
    assert!(authorizer.authorize_with_volumes(&f.run).await.is_err());
    assert_eq!(f.evidence().await, revoked);
    let audited_denials: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events WHERE actor_id=$1 AND decision='deny'",
    )
    .bind(f.actor.maintainer.as_uuid())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(audited_denials > 0);
    drop(authorizer);
    f.worker.close().await;
    println!(
        "REAL_CANONICAL_LAUNCH normal_original_caller=1 execute=1 source=1 active_user=1 issued_drain=1 readonly_credentials=1 mount_grants_separate=1 no_vm=1"
    );
}

async fn deny_withdrawn_role(
    f: &Fixture,
    authorizer: &super::CanonicalRunLaunchAuthorizer,
    project: uuid::Uuid,
) {
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(project)
        .bind(f.actor.maintainer.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(authorizer.authorize_with_volumes(&f.run).await.is_err());
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(project)
        .bind(f.actor.maintainer.as_uuid())
        .execute(&f.pool)
        .await
        .unwrap();
    authorizer.authorize_with_volumes(&f.run).await.unwrap();
}
