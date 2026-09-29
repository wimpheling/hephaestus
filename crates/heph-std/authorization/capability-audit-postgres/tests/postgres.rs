//! Opt-in real-`PostgreSQL` coverage for capability audit evidence and RLS.

use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use capability_audit::{
    CapabilityAuditContext, CapabilityAuditCursor, CapabilityAuditError, CapabilityAuditPage,
    CapabilityAuditReason, CapabilityAuditRepository, CapabilityDecision, CapabilityUseOutcome,
    NewCapabilityAuditEvent,
};
use capability_audit_postgres::PostgresCapabilityAuditRepository;
use capability_domain::{
    AuthorizationSnapshotId, CapabilityBindingId, CapabilityOperation, RuntimeSessionId,
};
use identity_domain::RequestId;
use runtime_types::RunId;
use serial_test::serial;
use std::collections::HashSet;

#[path = "postgres/support.rs"]
mod support;
use support::{audit_decision, seed, test_pool};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

#[tokio::test]
#[serial]
async fn records_redacted_exact_evidence_and_authorizes_inspection() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed(&pool).await;
    let repository = PostgresCapabilityAuditRepository::new(pool.clone());
    let started_at = OffsetDateTime::now_utc();
    let context = CapabilityAuditContext {
        runtime_session_id: RuntimeSessionId::from_uuid(fixture.session_id),
        snapshot_id: AuthorizationSnapshotId::from_uuid(fixture.snapshot_id),
        binding_id: CapabilityBindingId::from_uuid(fixture.binding_id),
        operation: CapabilityOperation::Inspect,
        request_id: RequestId::new(),
        authorization_model_version: AUTHORIZATION_MODEL_VERSION,
    };
    repository
        .append(&NewCapabilityAuditEvent::decision(
            context,
            CapabilityDecision::Allow,
            None,
            started_at,
        ))
        .await
        .expect("record exact authorization decision");
    repository
        .append(&NewCapabilityAuditEvent::capability_use(
            context,
            CapabilityUseOutcome::Succeeded,
            Some(CapabilityAuditReason::parse("completed").expect("safe reason")),
            started_at + Duration::milliseconds(1),
        ))
        .await
        .expect("record exact capability use");

    let first_page = repository
        .list_for_run(
            &fixture.owner,
            RunId::from_uuid(fixture.run_id),
            CapabilityAuditPage::new(1, None).expect("bounded page"),
        )
        .await
        .expect("authorized audit inspection");
    assert_eq!(first_page.len(), 1);
    assert_eq!(first_page[0].outcome, Some(CapabilityUseOutcome::Succeeded));
    assert_eq!(first_page[0].resource.id, fixture.repository_id);
    assert_eq!(first_page[0].slot.as_str(), "source");

    let second_page = repository
        .list_for_run(
            &fixture.owner,
            RunId::from_uuid(fixture.run_id),
            CapabilityAuditPage::new(
                10,
                Some(CapabilityAuditCursor {
                    occurred_at: first_page[0].occurred_at,
                    id: first_page[0].id,
                }),
            )
            .expect("cursor page"),
        )
        .await
        .expect("second audit page");
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page[0].decision, Some(CapabilityDecision::Allow));

    let mut session_tx = begin_actor_transaction(&pool, &fixture.owner)
        .await
        .expect("authorized session inspection transaction");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *session_tx)
        .await
        .expect("use non-bypass application role");
    let sessions: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT id, run_id, status
         FROM inspect_runtime_authority_sessions($1, 10)",
    )
    .bind(fixture.instance_id)
    .fetch_all(&mut *session_tx)
    .await
    .expect("inspect redacted runtime sessions");
    assert_eq!(
        sessions,
        vec![(fixture.session_id, fixture.run_id, String::from("active"))]
    );
    session_tx
        .commit()
        .await
        .expect("session inspection commit");

    assert_eq!(
        repository
            .list_for_run(
                &fixture.outsider,
                RunId::from_uuid(fixture.run_id),
                CapabilityAuditPage::new(10, None).expect("bounded page"),
            )
            .await,
        Err(CapabilityAuditError::Unavailable)
    );

    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name
         FROM information_schema.columns
         WHERE table_schema = 'public'
           AND table_name = 'capability_audit_inspection'
         ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect redacted view schema");
    for forbidden in ["credential", "payload", "path", "secret", "response"] {
        assert!(
            columns.iter().all(|column| !column.contains(forbidden)),
            "inspection schema exposed forbidden field family {forbidden}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn multikey_audit_pages_remain_stable_across_concurrent_writes() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed(&pool).await;
    let repository = PostgresCapabilityAuditRepository::new(pool);
    let base = OffsetDateTime::now_utc() - Duration::days(1);
    let id_prefix = Uuid::new_v4().as_u128() & !0xff;
    let id_100 = Uuid::from_u128(id_prefix + 100);
    let id_90 = Uuid::from_u128(id_prefix + 90);
    let id_80 = Uuid::from_u128(id_prefix + 80);
    let id_70 = Uuid::from_u128(id_prefix + 70);
    let id_60 = Uuid::from_u128(id_prefix + 60);
    let id_40 = Uuid::from_u128(id_prefix + 40);

    for (id, offset) in [(id_100, 4), (id_80, 3), (id_60, 2), (id_40, 1)] {
        repository
            .append(&audit_decision(
                &fixture,
                id,
                base + Duration::seconds(offset),
            ))
            .await
            .expect("seed ordered capability audit event");
    }

    let first = repository
        .list_for_run(
            &fixture.owner,
            RunId::from_uuid(fixture.run_id),
            CapabilityAuditPage::new(2, None).expect("first page"),
        )
        .await
        .expect("first capability audit page");
    assert_eq!(
        first.iter().map(|event| event.id).collect::<Vec<_>>(),
        vec![id_100, id_80]
    );

    let (release_writer, writer_released) = tokio::sync::oneshot::channel();
    let writer_repository = repository.clone();
    let writer_fixture = fixture.clone();
    let writer = tokio::spawn(async move {
        writer_released
            .await
            .expect("release concurrent audit writer");
        for (id, offset) in [(id_70, 3), (id_90, 3)] {
            writer_repository
                .append(&audit_decision(
                    &writer_fixture,
                    id,
                    base + Duration::seconds(offset),
                ))
                .await
                .expect("append concurrent capability audit event");
        }
    });
    release_writer
        .send(())
        .expect("release concurrent audit writer");
    writer.await.expect("concurrent audit writer task");

    let mut observed = first;
    let second = repository
        .list_for_run(
            &fixture.owner,
            RunId::from_uuid(fixture.run_id),
            CapabilityAuditPage::new(
                2,
                Some(CapabilityAuditCursor {
                    occurred_at: observed[1].occurred_at,
                    id: observed[1].id,
                }),
            )
            .expect("second page"),
        )
        .await
        .expect("second capability audit page");
    assert_eq!(
        second.iter().map(|event| event.id).collect::<Vec<_>>(),
        vec![id_70, id_60]
    );
    let second_cursor = CapabilityAuditCursor {
        occurred_at: second[1].occurred_at,
        id: second[1].id,
    };
    observed.extend(second);

    let third = repository
        .list_for_run(
            &fixture.owner,
            RunId::from_uuid(fixture.run_id),
            CapabilityAuditPage::new(2, Some(second_cursor)).expect("third page"),
        )
        .await
        .expect("third capability audit page");
    assert_eq!(
        third.iter().map(|event| event.id).collect::<Vec<_>>(),
        vec![id_40]
    );
    observed.extend(third);

    let observed_ids: Vec<_> = observed.iter().map(|event| event.id).collect();
    let unique_ids: HashSet<_> = observed_ids.iter().copied().collect();
    assert_eq!(unique_ids.len(), observed_ids.len());
    assert_eq!(observed_ids, vec![id_100, id_80, id_70, id_60, id_40]);
}

#[tokio::test]
#[serial]
async fn rejects_forged_ceiling_and_mutation() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let fixture = seed(&pool).await;
    let repository = PostgresCapabilityAuditRepository::new(pool.clone());
    let context = CapabilityAuditContext {
        runtime_session_id: RuntimeSessionId::from_uuid(fixture.session_id),
        snapshot_id: AuthorizationSnapshotId::from_uuid(fixture.snapshot_id),
        binding_id: CapabilityBindingId::from_uuid(fixture.binding_id),
        operation: CapabilityOperation::GitRead,
        request_id: RequestId::new(),
        authorization_model_version: AUTHORIZATION_MODEL_VERSION,
    };
    assert!(
        repository
            .append(&NewCapabilityAuditEvent::decision(
                context,
                CapabilityDecision::Allow,
                None,
                OffsetDateTime::now_utc(),
            ))
            .await
            .is_err(),
        "operation outside the immutable binding must fail closed"
    );

    let valid = NewCapabilityAuditEvent::decision(
        CapabilityAuditContext {
            operation: CapabilityOperation::Inspect,
            ..context
        },
        CapabilityDecision::Deny,
        Some(CapabilityAuditReason::parse("live_revoked").expect("safe reason")),
        OffsetDateTime::now_utc(),
    );
    repository
        .append(&valid)
        .await
        .expect("record valid denial");
    assert!(
        sqlx::query("UPDATE capability_audit_events SET reason_code = 'changed' WHERE id = $1")
            .bind(valid.id)
            .execute(&pool)
            .await
            .is_err(),
        "audit evidence must be immutable even for the table owner"
    );
    assert!(
        sqlx::query("DELETE FROM capability_audit_events WHERE id = $1")
            .bind(valid.id)
            .execute(&pool)
            .await
            .is_err(),
        "audit evidence must not be deletable"
    );
}
