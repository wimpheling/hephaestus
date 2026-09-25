use crate::support::{LifecycleState, ObservedBroker, PausingBroker, SENTINEL, key, role_pool};
use authz_postgres::PostgresMelangeAuthorizer;
use secret_application::{BrokerRequest, BrokerStatus, RotateSecret, SecretServiceError};
use secret_domain::{SecretSlotKey, SecretValue, SecretVersionId};
use secret_postgres::SecretRuntimeService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use tokio::sync::Notify;
use uuid::Uuid;

// This phase keeps its ordered database assertions together to preserve the lifecycle scenario.
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub(super) async fn rotate_and_revoke(state: &mut LifecycleState) {
    let pool = state.pool.clone();
    let service = Arc::clone(&state.service);
    let instance_id = state.instance_id.expect("instance");
    let brokered_rule_id = state.brokered_rule_id.expect("rule");
    let owner = state.owner.clone();
    let secret_id = state.secret_id;
    let first_version = state.first_version;
    let raw_run_id = state.raw_run_id.expect("raw run");
    let raw_authority = state.raw_authority.as_ref().expect("raw authority");
    let pinned_run = state.pinned_run.expect("pinned run");
    let pinned_authority = state.pinned_authority.take().expect("pinned authority");
    let revoked_pinned_run = state.revoked_pinned_run.expect("revoked pinned run");
    let revoked_pinned_authority = state
        .revoked_pinned_authority
        .take()
        .expect("revoked authority");
    let pre_revoked_run = state.pre_revoked_run.expect("pre-revoked run");
    let pre_revoked_authority = state
        .pre_revoked_authority
        .as_ref()
        .expect("pre-revoked authority");
    let update_revision_id = state.update_revision_id.expect("update revision");
    let app_runtime = SecretRuntimeService::new(
        role_pool("hephaestus_app").await,
        role_pool("hephaestus_worker").await,
        EncryptedStore::new(
            LocalKeyProvider::new("test/v1", [("test/v1", [7_u8; 32])]).expect("app runtime keys"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let pre_adapter_observed = Arc::new(AtomicBool::new(false));
    let runtime = SecretRuntimeService::new(
        pool.clone(),
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new(
                "test/v1",
                [("test/v1", [7_u8; 32]), ("test/v2", [8_u8; 32])],
            )
            .expect("runtime fixture keys should validate"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );

    for (label, credential, run_id, slot, rule_id, destination) in [
        (
            "wrong credential",
            &pinned_authority.credential,
            pre_revoked_run,
            "model",
            brokered_rule_id,
            "api.example.test",
        ),
        (
            "wrong run",
            &pre_revoked_authority.credential,
            pinned_run,
            "model",
            brokered_rule_id,
            "api.example.test",
        ),
        (
            "wrong slot",
            &pre_revoked_authority.credential,
            pre_revoked_run,
            "other",
            brokered_rule_id,
            "api.example.test",
        ),
        (
            "wrong rule",
            &pre_revoked_authority.credential,
            pre_revoked_run,
            "model",
            Uuid::new_v4(),
            "api.example.test",
        ),
        (
            "wrong destination",
            &pre_revoked_authority.credential,
            pre_revoked_run,
            "model",
            brokered_rule_id,
            "other.example.test",
        ),
    ] {
        let before: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM brokered_secret_audit_events
              WHERE run_id = $1 AND rule_id = $2
                AND event_kind = 'authorization_decision' AND decision = 'deny'",
        )
        .bind(pre_revoked_run.as_uuid())
        .bind(brokered_rule_id)
        .fetch_one(&pool)
        .await
        .expect("read denial count before mismatch");
        let result = app_runtime
            .use_brokered(
                credential,
                &BrokerRequest {
                    run_id,
                    slot: SecretSlotKey::parse(slot).expect("mismatch slot should validate"),
                    destination: String::from(destination),
                    operation: String::from("https_v1"),
                    body: serde_json::to_vec(&json!({ "rule_id": rule_id }))
                        .expect("mismatch brokered HTTPS request"),
                },
                &ObservedBroker {
                    observed: Arc::clone(&pre_adapter_observed),
                },
            )
            .await;
        assert!(result.is_err(), "{label} must remain denied");
        assert!(
            !pre_adapter_observed.load(Ordering::SeqCst),
            "{label} must not invoke the broker adapter"
        );
        let after: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM brokered_secret_audit_events
              WHERE run_id = $1 AND rule_id = $2
                AND event_kind = 'authorization_decision' AND decision = 'deny'",
        )
        .bind(pre_revoked_run.as_uuid())
        .bind(brokered_rule_id)
        .fetch_one(&pool)
        .await
        .expect("read denial count after mismatch");
        assert_eq!(after, before, "{label} must not receive attribution");
    }
    sqlx::query(
        "UPDATE agent_instances
          SET active_revision_id = $2, state = 'updating', run_gate_open = false
          WHERE id = $1",
    )
    .bind(instance_id.as_uuid())
    .bind(update_revision_id.as_uuid())
    .execute(&pool)
    .await
    .expect("restore update gate for pinned lease regression");
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let paused_call = tokio::spawn({
        let runtime = runtime.clone();
        let credential = pinned_authority.credential;
        let adapter = PausingBroker {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        };
        async move {
            runtime
                .use_brokered(
                    &credential,
                    &BrokerRequest {
                        run_id: pinned_run,
                        slot: SecretSlotKey::parse("model").expect("slot should validate"),
                        destination: String::from("api.example.test"),
                        operation: String::from("https_v1"),
                        body: serde_json::to_vec(&json!({ "rule_id": brokered_rule_id }))
                            .expect("brokered HTTPS request"),
                    },
                    &adapter,
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("broker call reached adapter before rotation");

    let second = SecretVersionId::new();
    let third = SecretVersionId::new();
    let rotate_second = service.rotate(
        &owner,
        RotateSecret {
            command_key: key("rotate-a", second.as_uuid()),
            secret_id,
            expected_active_version_id: first_version,
            new_version_id: second,
            value: SecretValue::new(format!("{SENTINEL}-v2")).expect("replacement should validate"),
        },
    );
    let rotate_third = service.rotate(
        &owner,
        RotateSecret {
            command_key: key("rotate-b", third.as_uuid()),
            secret_id,
            expected_active_version_id: first_version,
            new_version_id: third,
            value: SecretValue::new(format!("{SENTINEL}-v3")).expect("replacement should validate"),
        },
    );
    let (second_result, third_result) = tokio::join!(rotate_second, rotate_third);
    assert_ne!(second_result.is_ok(), third_result.is_ok());
    assert!(
        matches!(second_result, Err(SecretServiceError::StaleActiveVersion))
            || matches!(third_result, Err(SecretServiceError::StaleActiveVersion))
    );
    let active_version: Uuid =
        sqlx::query_scalar("SELECT active_version_id FROM secrets WHERE id = $1")
            .bind(secret_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("active rotated version");
    let expected_rotated_value = if active_version == second.as_uuid() {
        format!("{SENTINEL}-v2")
    } else {
        assert_eq!(active_version, third.as_uuid());
        format!("{SENTINEL}-v3")
    };
    release.notify_one();
    let pinned_response = paused_call
        .await
        .expect("pinned broker task should join")
        .expect("rotation must not invalidate an in-flight pinned broker call");
    assert_eq!(pinned_response.status, BrokerStatus::Succeeded);
    let pinned_version: Uuid = sqlx::query_scalar(
        "SELECT secret_version_id FROM brokered_secret_lease_snapshots
          WHERE run_id = $1 AND rule_id = $2",
    )
    .bind(pinned_run.as_uuid())
    .bind(brokered_rule_id)
    .fetch_one(&pool)
    .await
    .expect("pinned HTTPS lease version");
    assert_eq!(pinned_version, first_version.as_uuid());

    let revoke_entered = Arc::new(Notify::new());
    let revoke_release = Arc::new(Notify::new());
    let revoked_lease_id = revoked_pinned_authority.leases[0].lease_id.as_uuid();
    let revoke_call = tokio::spawn({
        let runtime = runtime.clone();
        let credential = revoked_pinned_authority.credential;
        let adapter = PausingBroker {
            entered: Arc::clone(&revoke_entered),
            release: Arc::clone(&revoke_release),
        };
        async move {
            runtime
                .use_brokered(
                    &credential,
                    &BrokerRequest {
                        run_id: revoked_pinned_run,
                        slot: SecretSlotKey::parse("model").expect("slot should validate"),
                        destination: String::from("api.example.test"),
                        operation: String::from("https_v1"),
                        body: serde_json::to_vec(&json!({ "rule_id": brokered_rule_id }))
                            .expect("brokered HTTPS request"),
                    },
                    &adapter,
                )
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), revoke_entered.notified())
        .await
        .expect("second broker call reached adapter");
    sqlx::query("UPDATE secret_leases SET status = 'revoked' WHERE id = $1")
        .bind(revoked_lease_id)
        .execute(&pool)
        .await
        .expect("revoke exact in-flight lease");
    revoke_release.notify_one();
    assert!(matches!(
        revoke_call.await.expect("revoked broker task should join"),
        Err(SecretServiceError::Unavailable)
    ));
    let revoked_decision: (Uuid, String, Option<String>) = sqlx::query_as(
        "SELECT lease_snapshot_id, decision, reason_code
           FROM brokered_secret_audit_events
          WHERE run_id = $1 AND rule_id = $2
            AND event_kind = 'authorization_decision' AND decision = 'deny'",
    )
    .bind(revoked_pinned_run.as_uuid())
    .bind(brokered_rule_id)
    .fetch_one(&pool)
    .await
    .expect("revoked HTTPS deny audit");
    let revoked_snapshot_id: Uuid =
        sqlx::query_scalar("SELECT id FROM brokered_secret_lease_snapshots WHERE lease_id = $1")
            .bind(revoked_lease_id)
            .fetch_one(&pool)
            .await
            .expect("revoked HTTPS lease snapshot");
    assert_eq!(
        revoked_decision,
        (
            revoked_snapshot_id,
            String::from("deny"),
            Some(String::from("live_authorization_unavailable"))
        )
    );
    let revoked_substitution_uses: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_audit_events
          WHERE run_id = $1 AND rule_id = $2 AND event_kind = 'substitution_use'",
    )
    .bind(revoked_pinned_run.as_uuid())
    .bind(brokered_rule_id)
    .fetch_one(&pool)
    .await
    .expect("revoked HTTPS substitution audit");
    assert_eq!(
        revoked_substitution_uses, 0,
        "revoked HTTPS operation has no successful substitution use"
    );

    let pinned_before_rotation = runtime
        .receive_raw(
            &raw_authority.credential,
            raw_run_id,
            SecretSlotKey::parse("model").expect("slot should validate"),
        )
        .await
        .expect("existing lease remains pinned after rotation");
    assert_eq!(raw_authority.leases[0].version_id, first_version);
    assert_eq!(pinned_before_rotation.value.expose(), SENTINEL.as_bytes());

    // Preserve the values needed by the final post-revocation assertions.
    state.active_version = Some(active_version);
    state.expected_rotated_value = Some(expected_rotated_value);
}
