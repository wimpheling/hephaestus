use super::{
    observations::{held, preparation_events, rows},
    wake_fixture::Wake,
};
use run_domain::RunCleanupHostId;
use run_orchestrator::{
    RepositoryError, RunCleanupWakeOriginal, RunCleanupWakeRequest, RunRepository,
};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL gate"]
async fn default_public107_never_looks_up_private_schema(pool: PgPool) {
    crate::fixtures::private_migrations::checkpoint(107)
        .run(&pool)
        .await
        .unwrap();
    let before: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .unwrap();
    let private: bool = sqlx::query_scalar("SELECT to_regclass('run_cleanup_targets') IS NOT NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!private);
    let request = RunCleanupWakeRequest::new(
        runtime_types::RunId::new(),
        runtime_types::AgentInstanceId::new(),
        RunCleanupWakeOriginal {
            drain_attempt: Uuid::new_v4(),
            drain_command: Uuid::new_v4(),
            removal_id: Uuid::new_v4(),
            command_key: [3; 32],
            input_hash: [4; 32],
            expected_version: 1,
            creator: Uuid::new_v4(),
            request: Uuid::new_v4(),
        },
    )
    .unwrap();
    let scope = RunCleanupHostId::new("owner".into(), "host".into()).unwrap();
    let repository = run_postgres::PgRunRepository::new(pool.clone());
    assert!(matches!(
        repository.load_cleanup_wake(&request, &scope).await,
        Err(RepositoryError::InvalidData(
            "owning cleanup wake qualification is unsupported"
        ))
    ));
    assert!(matches!(
        repository.prepare_cleanup_wake(&request, &scope).await,
        Err(RepositoryError::InvalidData(
            "owning cleanup wake preparation is unsupported"
        ))
    ));
    let after: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version,checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    let runs: i64 = sqlx::query_scalar("SELECT count(*) FROM runs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(runs, 0);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL and native ext4 gate"]
async fn actual_app_and_default_owner_deny_with_public_execute_revoked(pool: PgPool) {
    let h = Wake::new(&pool, 0).await;
    let before = rows(&pool).await;
    // ROLE none resets to the default owner; PUBLIC is an ACL grantee only.
    for role in ["hephaestus_app", "none"] {
        for facade in [
            "load_run_cleanup_wake118",
            "run_cleanup_wake_reservations118",
            "lock_run_cleanup_wake118",
        ] {
            let mut tx = pool.begin().await.unwrap();
            let role_query = match role {
                "hephaestus_app" => sqlx::query("SET LOCAL ROLE hephaestus_app"),
                "none" => sqlx::query("SET LOCAL ROLE none"),
                _ => unreachable!("closed role matrix"),
            };
            role_query.execute(&mut *tx).await.unwrap();
            let query = match facade {
                "load_run_cleanup_wake118" => sqlx::query(
                    "SELECT * FROM load_run_cleanup_wake118($1,$2,'{}','missing','missing',NULL)",
                ),
                "run_cleanup_wake_reservations118" => sqlx::query(
                    "SELECT * FROM run_cleanup_wake_reservations118($1,$2,'{}','missing','missing',NULL)",
                ),
                "lock_run_cleanup_wake118" => sqlx::query(
                    "SELECT * FROM lock_run_cleanup_wake118($1,$2,'{}','missing','missing',NULL)",
                ),
                _ => unreachable!("closed facade matrix"),
            };
            let error = query
                .bind(Uuid::new_v4())
                .bind(Uuid::new_v4())
                .fetch_all(&mut *tx)
                .await
                .unwrap_err();
            assert_eq!(super::observations::code(&error).as_deref(), Some("42501"));
            tx.rollback().await.unwrap();
        }
    }
    let public_execute: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_proc p CROSS JOIN LATERAL aclexplode(p.proacl) acl WHERE p.proname IN ('load_run_cleanup_wake118','run_cleanup_wake_reservations118','lock_run_cleanup_wake118') AND acl.grantee=0 AND acl.privilege_type='EXECUTE')").fetch_one(&pool).await.unwrap();
    assert!(!public_execute);
    let app = run_postgres::PgRunRepository::new(h.fixture.birth.app.clone())
        .with_execution_profile(run_domain::FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(h.fixture.scope.clone())
        .with_qualified_cleanup_wake(h.fixture.scope.clone());
    assert!(matches!(
        app.load_cleanup_wake(&h.request, &h.fixture.scope).await,
        Err(RepositoryError::InvalidData(
            "cleanup wake requires the configured worker role"
        ))
    ));
    assert_eq!(rows(&pool).await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL and native ext4 gate"]
async fn owning_preparation_freezes_full_set_after_exact_cancel_and_survives_withdrawal(
    pool: PgPool,
) {
    let h = Wake::new(&pool, 2).await;
    let before = rows(&pool).await;
    held(
        &h.repository()
            .load_cleanup_wake(&h.request, &h.fixture.scope)
            .await
            .unwrap_err(),
    );
    assert_eq!(rows(&pool).await, before);
    let prepared = h.prepare().await;
    let after = rows(&pool).await;
    let mut unchanged = after.clone();
    for table in ["run_cleanup_targets", "run_cleanup_target_leases"] {
        unchanged[table] = before[table].clone();
    }
    preparation_events(&pool, &before, &after, h.request.run_id(), 2).await;
    for table in ["run_events", "application_events", "product_event_outbox"] {
        unchanged[table] = before[table].clone();
    }
    // Owning preparation may only close acquisition; every other Run field stays exact.
    normalize_owned_acquisition(&mut unchanged, &before, h.request.run_id());
    assert_eq!(unchanged, before);
    assert_eq!(
        after["run_cleanup_target_leases"].as_array().unwrap().len(),
        2
    );
    assert_eq!(h.read().await, prepared);
    assert_eq!(h.prepare().await, prepared);
    assert_eq!(rows(&pool).await, after);
    h.withdraw().await;
    assert_eq!(h.read().await, prepared);
    assert_eq!(h.prepare().await, prepared);
    h.fixture.physical.assert_unchanged();
    h.fixture.birth.assert_historical_migrations(&pool).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL and native ext4 gate"]
async fn exact_processed_cancel_is_required_and_mismatched_delivery_never_qualifies(pool: PgPool) {
    let h = Wake::from_fixture(crate::fixture::Fixture::new(&pool, 0).await, true, false).await;
    let before = rows(&pool).await;
    held(
        &h.repository()
            .prepare_cleanup_wake(&h.request, &h.fixture.scope)
            .await
            .unwrap_err(),
    );
    assert_eq!(rows(&pool).await, before);
    let mut substituted = h.cancel.clone();
    substituted.reason = "substituted reason".into();
    h.fixture
        .repository()
        .request_cancel(&substituted)
        .await
        .unwrap();
    let mismatch = rows(&pool).await;
    held(
        &h.repository()
            .prepare_cleanup_wake(&h.request, &h.fixture.scope)
            .await
            .unwrap_err(),
    );
    assert_eq!(rows(&pool).await, mismatch);
    // Existing idempotent inbox retains contradictory same-ID history.
    assert!(
        !h.fixture
            .repository()
            .request_cancel(&h.cancel)
            .await
            .unwrap()
    );
    assert_eq!(rows(&pool).await, mismatch);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL and native ext4 gate"]
async fn open_instance_and_missing_target_are_readonly_held_then_owner_prepares(pool: PgPool) {
    let fixture = crate::fixture::Fixture::new(&pool, 0).await;
    fixture.consume().await.unwrap();
    let h = Wake::from_fixture(fixture, false, true).await;
    let before = rows(&pool).await;
    held(
        &h.repository()
            .load_cleanup_wake(&h.request, &h.fixture.scope)
            .await
            .unwrap_err(),
    );
    assert_eq!(rows(&pool).await, before);
    let target = h.prepare().await;
    assert_eq!(h.read().await, target);
    let after = rows(&pool).await;
    let mut unchanged = after.clone();
    preparation_events(&pool, &before, &after, h.request.run_id(), 0).await;
    normalize_owned_acquisition(&mut unchanged, &before, h.request.run_id());
    assert_eq!(unchanged["runs"], before["runs"]);
    assert!(after["run_cleanup_receipts"].as_array().unwrap().is_empty());
    assert!(
        after["run_cleanup_completions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires explicit private PostgreSQL and native ext4 gate"]
async fn genuine115_cancel_before117_and_named_legacy_composition_remain_held(pool: PgPool) {
    let h = Wake::historical(crate::fixture::Fixture::new(&pool, 0).await, false, true).await;
    let before = rows(&pool).await;
    super::wake_fixture::historical_held(&h.fixture, &h.request).await;
    let legacy = h
        .repository()
        .with_execution_profile(run_domain::FreshRunExecutionProfile::LegacyScalar);
    assert!(matches!(
        legacy
            .prepare_cleanup_wake(&h.request, &h.fixture.scope)
            .await,
        Err(RepositoryError::InvalidData(
            "owning cleanup wake is unsupported for this composition"
        ))
    ));
    assert_eq!(rows(&pool).await, before);
    h.upgrade().await;
    let accepted: bool =
        sqlx::query_scalar("SELECT invocation119_remove_resume_matches($1,$2,$3,$4)")
            .bind(h.request.run_id().as_uuid())
            .bind(h.fixture.scope.provider_namespace())
            .bind(h.fixture.scope.host_id())
            .bind(serde_json::to_value(h.fixture.birth.input.source().platform()).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!accepted, "pre117 cannot acquire fresh resume lineage");
    assert_eq!(rows(&pool).await, before);
    assert!(
        before["run_invocation_consumptions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Final28 read remains held until the actual owner freezes the zero set.
    held(
        &h.repository()
            .load_cleanup_wake(&h.request, &h.fixture.scope)
            .await
            .unwrap_err(),
    );
    assert_eq!(rows(&pool).await, before);
    let prepared = h.prepare().await;
    assert_eq!(prepared.request, h.request);
    assert_eq!(prepared.run.state, run_domain::RunState::Queued);
    assert_eq!(prepared.run.outcome, None);
    assert_eq!(h.read().await, prepared);
    let after = rows(&pool).await;
    assert_eq!(after["run_cleanup_targets"].as_array().unwrap().len(), 1);
    assert_eq!(after["run_cleanup_targets"][0]["generation"], 1);
    assert_eq!(
        after["run_cleanup_targets"][0]["run_id"],
        serde_json::json!(h.request.run_id())
    );
    let mut unchanged = after.clone();
    preparation_events(&pool, &before, &after, h.request.run_id(), 0).await;
    for table in [
        "run_cleanup_targets",
        "run_cleanup_target_leases",
        "run_events",
        "application_events",
        "product_event_outbox",
    ] {
        unchanged[table] = before[table].clone();
    }
    normalize_owned_acquisition(&mut unchanged, &before, h.request.run_id());
    assert_eq!(unchanged, before);
    assert_eq!(
        after["run_cleanup_target_leases"].as_array().unwrap().len(),
        0
    );
    assert_eq!(h.prepare().await, prepared);
    assert_eq!(rows(&pool).await, after);
    h.fixture.physical.assert_unchanged();
}

fn owned_run_index(snapshot: &serde_json::Value, run: runtime_types::RunId) -> usize {
    let id = run.to_string();
    let mut matches = snapshot["runs"]
        .as_array()
        .expect("complete Run snapshot is an array")
        .iter()
        .enumerate()
        .filter(|(_, row)| row["id"].as_str() == Some(id.as_str()))
        .map(|(index, _)| index);
    let index = matches.next().expect("actual owned Run exists in snapshot");
    assert!(matches.next().is_none(), "actual owned Run is unique");
    index
}

fn normalize_owned_acquisition(
    after: &mut serde_json::Value,
    before: &serde_json::Value,
    run: runtime_types::RunId,
) {
    let before_index = owned_run_index(before, run);
    let after_index = owned_run_index(after, run);
    let prior = &before["runs"][before_index]["volume_acquisition_closed_at"];
    assert!(prior.is_null(), "actual owned acquisition was open");
    assert!(
        after["runs"][after_index]["volume_acquisition_closed_at"]
            .as_str()
            .is_some(),
        "actual owned acquisition is now durably closed"
    );
    after["runs"][after_index]["volume_acquisition_closed_at"] = prior.clone();
}
