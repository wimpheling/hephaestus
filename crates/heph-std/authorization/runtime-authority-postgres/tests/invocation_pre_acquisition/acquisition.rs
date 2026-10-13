use super::fixture::{Fixture, code, pg_time};
use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use capability_domain::{
    RuntimeCredential, RuntimeCredentialGeneration, RuntimeInvocation, RuntimeSessionId,
    RuntimeSessionIdentity,
};
use run_domain::RunState;
use run_orchestrator::{RunCleanupRepository, RunRepository};
use runtime_authority::{NewRuntimeSession, RuntimeSessionRepository};
use sqlx::PgPool;
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn actual_acquisition_provisioning_and_runtime_history_never_reopen_first_acquisition(
    pool: PgPool,
) {
    let f = Box::pin(Fixture::new(&pool, 2)).await;
    f.allowed().await;
    let selections = f
        .birth
        .volumes()
        .load_run_selections(f.run.id)
        .await
        .unwrap();
    let now = pg_time();
    let leases = f
        .birth
        .volumes()
        .acquire_run(
            &selections,
            f.birth.scope.host_id(),
            now,
            now + time::Duration::hours(2),
        )
        .await
        .unwrap();
    assert_eq!(leases.len(), 2);
    let acquired = f.birth.repository().get(f.run.id).await.unwrap();
    assert_eq!(acquired.state, RunState::LeasingVolume);
    f.denied(&acquired).await;
    assert!(
        !f.raw(
            &f.birth.birth.worker,
            &acquired,
            &f.config(),
            AUTHORIZATION_MODEL_VERSION
        )
        .await
        .unwrap()
    );
    assert!(!f.b2(&acquired, "leasing_volume").await);
    let before = f.rows().await;
    for query in [
        sqlx::query("UPDATE runs SET volume_selection_snapshot=NULL WHERE id=$1"),
        sqlx::query("UPDATE agent_instance_volume_leases SET released_at=now() WHERE run_id=$1"),
    ] {
        let error = query
            .bind(f.run.id.as_uuid())
            .execute(&f.birth.birth.worker)
            .await
            .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("23000"));
    }
    assert_eq!(
        f.rows().await,
        before,
        "no fabricated released history or snapshot reset"
    );
    f.birth
        .repository()
        .bind_resources(f.run.id, None, None, None, &f.birth.vm.0)
        .await
        .unwrap();
    let provisioning = f
        .birth
        .repository()
        .transition(f.run.id, RunState::Provisioning, None, None)
        .await
        .unwrap();
    let before = f.rows().await;
    assert!(
        f.b2(&provisioning, "provisioning").await,
        "full B2 at actual Provisioning before provider IO"
    );
    assert_eq!(f.rows().await, before);
    f.birth.physical.assert_unchanged();
    f.denied(&provisioning).await;
    late_phase_history(&f, &provisioning).await;
}

async fn late_phase_history(f: &Fixture, provisioning: &run_domain::Run) {
    let selections = f
        .birth
        .volumes()
        .load_run_selections(f.run.id)
        .await
        .unwrap();
    // These are protected worker authority/ACK/fence metadata, not guest or VM proof.
    let snapshot = f
        .repository
        .resolve_snapshot(provisioning, AUTHORIZATION_MODEL_VERSION)
        .await
        .unwrap();
    let issued = pg_time();
    let identity = RuntimeSessionIdentity::new(
        RuntimeSessionId::from_uuid(f.run.id.as_uuid()),
        snapshot.principal(),
        RuntimeInvocation::Run(f.run.id),
        &snapshot,
        issued,
        issued + time::Duration::minutes(10),
    )
    .unwrap();
    let credential = RuntimeCredential::from_secret([41; 32])
        .storage_hash(identity.id(), RuntimeCredentialGeneration::INITIAL);
    let pending = f
        .repository
        .create(NewRuntimeSession {
            snapshot: &snapshot,
            identity: &identity,
            generation: RuntimeCredentialGeneration::INITIAL,
            credential_hash: credential,
            attachment_id: None,
        })
        .await
        .unwrap();
    f.repository
        .acknowledge(pending.id, RuntimeCredentialGeneration::INITIAL, pg_time())
        .await
        .unwrap();
    f.birth
        .volumes()
        .mark_run_attached(
            &selections,
            f.birth.scope.host_id(),
            pg_time(),
            pg_time() + time::Duration::hours(2),
        )
        .await
        .unwrap();
    for phase in [RunState::Starting, RunState::Running] {
        let run = f
            .birth
            .repository()
            .transition(f.run.id, phase, None, None)
            .await
            .unwrap();
        f.denied(&run).await;
        assert!(
            !f.raw(
                &f.birth.birth.worker,
                &run,
                &f.config(),
                AUTHORIZATION_MODEL_VERSION
            )
            .await
            .unwrap()
        );
    }
    let failed = f
        .birth
        .repository()
        .transition(
            f.run.id,
            RunState::Failed,
            None,
            Some("metadata-only test failure"),
        )
        .await
        .unwrap();
    f.denied(&failed).await;
    let cleanup = run_postgres::PgRunCleanupRepository::new(
        f.birth.birth.worker.clone(),
        f.birth.scope.clone(),
    );
    cleanup.begin_cleanup(f.run.id).await.unwrap();
    assert!(
        cleanup
            .recorded_cleanup_receipt(f.run.id)
            .await
            .unwrap()
            .is_none()
    );
    let history:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM run_cleanup_targets WHERE run_id=$1),(SELECT count(*) FROM run_cleanup_target_leases WHERE run_id=$1),(SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL)")
        .bind(f.run.id.as_uuid()).fetch_one(&f.birth.pool).await.unwrap();
    assert_eq!(history, (1, 2, 2));
    f.denied(&failed).await;
    f.birth.physical.assert_unchanged();
}
