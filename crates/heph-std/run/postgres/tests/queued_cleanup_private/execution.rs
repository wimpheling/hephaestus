//! Cleanup qualification never grants delayed execution, leases or credentials.
use super::{
    controller::{Control, build},
    observations::{code, rows, unconsumed},
    queued_fixture::Queued,
};
use run_domain::RunState;
use run_orchestrator::{
    OrchestratorError, RepositoryError, RunRepository, RunRuntimeCatalog, RunRuntimeCatalogError,
};
use sqlx::PgPool;
use volume_trait::{RunVolumeMetadataRepository, VolumeError};

pub fn repository_code(error: &RepositoryError, expected: &str) {
    let RepositoryError::Storage(source) = error else {
        panic!("expected storage denial: {error:?}")
    };
    assert_eq!(
        code(source.downcast_ref::<sqlx::Error>().unwrap()).as_deref(),
        Some(expected)
    );
}
#[sqlx::test(migrations = false)]
async fn delayed_start_runtime_acquisition_renewal_session_and_ack_never_gain_authority(
    pool: PgPool,
) {
    let h = Queued::new(&pool, 2, true).await;
    let prepared = h.prepare().await;
    let before = rows(&pool).await;
    repository_code(&h.fixture.consume().await.unwrap_err(), "P1171");
    assert_eq!(rows(&pool).await, before);
    let control = Control::new();
    let core = build(&h, control.clone(), None, None);
    let error = core.start_run(&h.fixture.start).await.unwrap_err();
    let OrchestratorError::Repository(error) = error else {
        panic!("actual Start admission: {error:?}")
    };
    repository_code(&error, "P1171");
    assert_eq!(rows(&pool).await, before);
    assert!(control.log.lock().unwrap().is_empty());
    for next in [
        RunState::LeasingVolume,
        RunState::Provisioning,
        RunState::Starting,
        RunState::Running,
        RunState::Succeeded,
        RunState::Failed,
        RunState::CleanedUp,
    ] {
        transition_denied(&h, next, None, None).await;
        assert_eq!(rows(&pool).await, before);
    }
    transition_denied(
        &h,
        RunState::Cancelled,
        Some(&vm_trait::VmExit {
            code: Some(0),
            signal: None,
        }),
        None,
    )
    .await;
    transition_denied(
        &h,
        RunState::Cancelled,
        None,
        Some("invented execution failure"),
    )
    .await;
    let error = h
        .repository()
        .load_runtime(&prepared.run)
        .await
        .unwrap_err();
    let RunRuntimeCatalogError::Storage(source) = error else {
        panic!("actual runtime denial: {error:?}")
    };
    assert_eq!(
        code(source.downcast_ref::<sqlx::Error>().unwrap()).as_deref(),
        Some("P1190")
    );
    let value: serde_json::Value = sqlx::query_scalar("SELECT invocation119_selections($1)")
        .bind(h.request.run_id().as_uuid())
        .fetch_one(&h.fixture.birth.worker)
        .await
        .unwrap();
    // Genuine immutable115 selections are comparison DATA, never an admission.
    let identity = volume_domain::RunVolumeIdentity::new(
        prepared.run.id,
        prepared.run.instance_id,
        prepared.run.instance_revision_id,
        prepared.run.release_id,
        prepared.run.release_agent_id,
        h.fixture.birth.fixture.consuming_project,
    )
    .unwrap();
    assert_eq!(serde_json::to_value(identity).unwrap(), value["identity"]);
    let selected = value["selections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            assert_eq!(row["identity"], value["identity"]);
            assert_eq!(row["origin"], "explicit");
            volume_domain::RunVolumeSelection::new(
                identity,
                serde_json::from_value(row["scope"].clone()).unwrap(),
                serde_json::from_value(row["declaration"].clone()).unwrap(),
                volume_domain::VolumeSelectionOrigin::Explicit,
            )
            .unwrap()
        })
        .collect();
    let selections = volume_domain::RunVolumeSelections::new(identity, selected).unwrap();
    assert_eq!(serde_json::to_value(&selections).unwrap(), value);
    assert_eq!(selections.identity().run_id(), h.request.run_id());
    let now = time::OffsetDateTime::now_utc();
    let expires = now + time::Duration::minutes(5);
    for error in [
        h.fixture
            .volumes()
            .load_run_selections(h.request.run_id())
            .await
            .unwrap_err(),
        h.fixture
            .volumes()
            .acquire_run(&selections, h.fixture.scope.host_id(), now, expires)
            .await
            .unwrap_err(),
        h.fixture
            .volumes()
            .heartbeat_run(&selections, h.fixture.scope.host_id(), now, expires)
            .await
            .unwrap_err(),
    ] {
        let VolumeError::Metadata(source) = error else {
            panic!("actual volume denial: {error:?}")
        };
        assert_eq!(
            code(source.downcast_ref::<sqlx::Error>().unwrap()).as_deref(),
            Some("P1190")
        );
        assert_eq!(rows(&pool).await, before);
    }
    for issue in [true, false] {
        let mut tx = h.fixture.birth.worker.begin().await.unwrap();
        let config = serde_json::to_vec(&h.fixture.config.canonical_json()).unwrap();
        let error = if issue {
            sqlx::query("SELECT write_invocation119_session($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
                .bind(h.request.run_id().as_uuid())
                .bind(config)
                .bind("test-model")
                .bind(vec![7_u8; 32])
                .bind(vec![8_u8; 32])
                .bind(1_i64)
                .bind(vec![9_u8; 32])
                .bind(now)
                .bind(expires)
                .bind(now.unix_timestamp_nanos().to_string())
                .bind(expires.unix_timestamp_nanos().to_string())
                .execute(&mut *tx)
                .await
                .unwrap_err()
        } else {
            sqlx::query("SELECT acknowledge_invocation119_session($1,$2,$3,$4)")
                .bind(h.request.run_id().as_uuid())
                .bind(config)
                .bind(1_i64)
                .bind(now)
                .execute(&mut *tx)
                .await
                .unwrap_err()
        };
        assert_eq!(code(&error).as_deref(), Some("P1190"));
        tx.rollback().await.unwrap();
        assert_eq!(rows(&pool).await, before);
    }
    let mut tx = h.fixture.birth.worker.begin().await.unwrap();
    let error =
        sqlx::query("UPDATE runs SET state='running',state_version=state_version+1 WHERE id=$1")
            .bind(h.request.run_id().as_uuid())
            .execute(&mut *tx)
            .await
            .unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23000"));
    tx.rollback().await.unwrap();
    assert_eq!(rows(&pool).await, before);
    unconsumed(&h).await;
    h.fixture.physical.assert_unchanged();
}
async fn transition_denied(
    h: &Queued,
    next: RunState,
    exit: Option<&vm_trait::VmExit>,
    failure: Option<&str>,
) {
    assert!(matches!(
        h.repository()
            .transition(h.request.run_id(), next, exit, failure)
            .await,
        Err(RepositoryError::InvalidData(
            "queued cleanup only permits cancellation and cleanup without exit or failure"
        ))
    ));
}
