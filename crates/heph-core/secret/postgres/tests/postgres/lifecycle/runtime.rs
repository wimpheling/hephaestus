use crate::support::{
    AcceptingBroker, FailingBroker, FakeBroker, LifecycleState, ObservedBroker,
    ObservedVerifiedBroker, SENTINEL,
};
use authz_postgres::PostgresMelangeAuthorizer;
use runtime_types::RunId;
use secret_application::{
    BrokerRequest, BrokerStatus, SecretServiceError, VerifiedBrokeredHttpsRule,
};
use secret_domain::SecretSlotKey;
use secret_postgres::SecretRuntimeService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use serde_json::json;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

// This phase keeps its ordered database assertions together to preserve the lifecycle scenario.
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
pub(super) async fn exercise_brokered_runtime(state: &LifecycleState) {
    let pool = state.pool.clone();
    let authority = state.authority.as_ref().expect("resolved authority");
    let run_id = state.run_id.expect("run");
    let brokered_rule_id = state.brokered_rule_id.expect("rule");

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
    let adapter = FakeBroker {
        observed: AtomicBool::new(false),
        expected_credential: SENTINEL.as_bytes().to_vec(),
    };
    let broker_response = runtime
        .use_brokered(
            &authority.credential,
            &BrokerRequest {
                run_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                destination: String::from("api.example.test"),
                operation: String::from("complete"),
                body: b"bounded request".to_vec(),
            },
            &adapter,
        )
        .await
        .expect("broker should exercise host-only credential");
    assert_eq!(broker_response.status, BrokerStatus::Succeeded);
    assert!(
        !broker_response
            .body
            .windows(SENTINEL.len())
            .any(|value| value == SENTINEL.as_bytes())
    );
    assert!(adapter.observed.load(Ordering::SeqCst));
    let https_body = serde_json::to_vec(&json!({ "rule_id": brokered_rule_id }))
        .expect("brokered HTTPS request");
    let verified_rule_observed = Arc::new(Mutex::new(None));
    let verified_rule_adapter = ObservedVerifiedBroker {
        observed: Arc::clone(&verified_rule_observed),
    };
    let https_response = runtime
        .use_brokered(
            &authority.credential,
            &BrokerRequest {
                run_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                destination: String::from("api.example.test"),
                operation: String::from("https_v1"),
                body: https_body.clone(),
            },
            &verified_rule_adapter,
        )
        .await
        .expect("exact brokered HTTPS snapshot should authorize");
    assert_eq!(https_response.status, BrokerStatus::Succeeded);
    assert_eq!(
        verified_rule_observed
            .lock()
            .expect("verified rule lock")
            .as_ref(),
        Some(&VerifiedBrokeredHttpsRule {
            rule_id: brokered_rule_id,
            destination_origin: String::from("https://api.example.test"),
            header_name: String::from("authorization"),
            header_prefix: Some(String::from("Bearer ")),
        })
    );
    let actual_calls: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_audit_events decision
         JOIN brokered_secret_audit_events outcome ON outcome.request_id = decision.request_id
         WHERE decision.run_id = $1 AND decision.event_kind = 'authorization_decision'
           AND decision.decision = 'allow' AND outcome.event_kind = 'substitution_use'
           AND outcome.outcome = 'succeeded'",
    )
    .bind(run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("real HTTPS correlated audit");
    assert_eq!(actual_calls, 1);
    let failed = runtime
        .use_brokered(
            &authority.credential,
            &BrokerRequest {
                run_id,
                slot: SecretSlotKey::parse("model").expect("slot"),
                destination: String::from("api.example.test"),
                operation: String::from("https_v1"),
                body: https_body.clone(),
            },
            &FailingBroker,
        )
        .await;
    assert!(matches!(failed, Err(SecretServiceError::BrokerAdapter(_))));
    let failed_calls: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_audit_events WHERE run_id=$1 AND outcome='failed'",
    )
    .bind(run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("failed HTTPS evidence");
    assert_eq!(failed_calls, 1);
    let denied = runtime
        .use_brokered(
            &authority.credential,
            &BrokerRequest {
                run_id,
                slot: SecretSlotKey::parse("model").expect("slot"),
                destination: String::from("other.example.test"),
                operation: String::from("https_v1"),
                body: https_body.clone(),
            },
            &AcceptingBroker,
        )
        .await;
    assert!(matches!(
        denied,
        Err(SecretServiceError::BrokerRequestDenied)
    ));
    let denied_calls: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM brokered_secret_audit_events WHERE run_id=$1 AND decision='deny'",
    )
    .bind(run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("denied HTTPS evidence");
    assert_eq!(denied_calls, 1);
    let wrong_rule_observed = Arc::new(AtomicBool::new(false));
    let wrong_rule_result = runtime
        .use_brokered(
            &authority.credential,
            &BrokerRequest {
                run_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                destination: String::from("api.example.test"),
                operation: String::from("https_v1"),
                body: serde_json::to_vec(&json!({ "rule_id": Uuid::new_v4() }))
                    .expect("unbound rule request"),
            },
            &ObservedBroker {
                observed: Arc::clone(&wrong_rule_observed),
            },
        )
        .await;
    assert!(matches!(
        wrong_rule_result,
        Err(SecretServiceError::BrokerRequestDenied)
    ));
    assert!(!wrong_rule_observed.load(Ordering::SeqCst));
    let revoked_run_result = runtime
        .use_brokered(
            &authority.credential,
            &BrokerRequest {
                run_id: RunId::new(),
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                destination: String::from("api.example.test"),
                operation: String::from("https_v1"),
                body: https_body,
            },
            &AcceptingBroker,
        )
        .await;
    assert!(matches!(
        revoked_run_result,
        Err(SecretServiceError::RuntimeAuthenticationDenied)
    ));
    let alternate_destination = runtime
        .use_brokered(
            &authority.credential,
            &BrokerRequest {
                run_id,
                slot: SecretSlotKey::parse("model").expect("slot should validate"),
                destination: String::from("other.example.test"),
                operation: String::from("complete"),
                body: Vec::new(),
            },
            &adapter,
        )
        .await;
    assert!(matches!(
        alternate_destination,
        Err(SecretServiceError::BrokerRequestDenied)
    ));
}
