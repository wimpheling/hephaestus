//! One genuine frozen initialization per `SQLx` child.
use heph_run::{RunAuthorityManager, RunRepository, RunState};
use run_orchestrator::{RunLaunchAuthorizer, RunSecretManager};
use secret_postgres::PostgresSecretMountMetadata;
use sqlx::PgPool;

pub type Fixture = crate::qualified_invocation::fixture::Fixture;

pub async fn running(pool: &PgPool, count: usize) -> Fixture {
    let f = Box::pin(Fixture::provisioning(pool, count)).await;
    for (pool, expected) in [
        (&f.birth.birth.app, "hephaestus_app"),
        (&f.birth.birth.worker, "hephaestus_worker"),
    ] {
        let role: String = sqlx::query_scalar("SELECT current_user::text")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(role, expected);
    }
    let provisioning = f.run().await;
    let bootstrap = f.issue().await;
    f.manager()
        .acknowledge(
            &provisioning,
            bootstrap.session_id(),
            bootstrap.generation(),
        )
        .await
        .unwrap();
    assert!(f.session().await.acknowledged_at.is_some());
    assert!(f.handoff_files().is_empty());
    f.attach().await;
    for state in [RunState::Starting, RunState::Running] {
        f.birth
            .repository()
            .transition(provisioning.id, state, None, None)
            .await
            .unwrap();
    }
    let running = f.run().await;
    f.manager().reauthorize(&running).await.unwrap();
    f.launcher().authorize_with_volumes(&running).await.unwrap();
    let secret = secrets(&f);
    assert!(secret.prepare(&running).await.unwrap().mounts.is_empty());
    secret.reauthorize(&running).await.unwrap();
    f.birth.physical.assert_unchanged();
    f
}

pub fn secrets(f: &Fixture) -> crate::secret_ports::Manager {
    crate::secret_ports::manager(
        PostgresSecretMountMetadata::new(f.birth.birth.worker.clone())
            .with_qualified_invocation_configuration(f.birth.config.clone()),
    )
}
