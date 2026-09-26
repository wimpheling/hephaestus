use super::*;

pub async fn wait_for_accepted_gateway_service_hold(
    pool: &sqlx::PgPool,
    fixture: &GatewayServiceGoldenFixture,
    instance_id: uuid::Uuid,
    fencing_token: i64,
    started_at: OffsetDateTime,
    baseline_invocations: i64,
) -> uuid::Uuid {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM gateway_invocations WHERE gateway_id = $1",
            )
            .bind(fixture.gateway_id)
            .fetch_one(pool)
            .await
            .expect("count accepted hold invocation");
            let row: Option<(uuid::Uuid, i64)> = sqlx::query_as(
                "SELECT invocation.id, invocation.service_instance_fencing_token
                   FROM gateway_invocations AS invocation
                   JOIN gateway_runtime_authority_sessions AS session
                     ON session.invocation_id = invocation.id
                    AND session.gateway_id = invocation.gateway_id
                    AND session.gateway_revision_id = invocation.gateway_revision_id
                  WHERE invocation.gateway_id = $1
                    AND invocation.gateway_revision_id = $2
                    AND invocation.service_instance_id = $3
                    AND invocation.outcome = 'accepted'
                    AND invocation.accepted_at >= $4
                    AND session.admission_mode = 'host_mediated'
                    AND session.status = 'active'
                  ORDER BY invocation.accepted_at DESC, invocation.id DESC
                  LIMIT 1",
            )
            .bind(fixture.gateway_id)
            .bind(fixture.revision_id)
            .bind(instance_id)
            .bind(started_at)
            .fetch_optional(pool)
            .await
            .expect("read accepted host-mediated hold invocation");
            if count == baseline_invocations + 1 {
                if let Some((invocation_id, observed_fencing_token)) = row {
                    assert_eq!(observed_fencing_token, fencing_token);
                    return invocation_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("A hold becomes an accepted host-mediated invocation")
}

pub async fn wait_for_gateway_service_cutover_state(
    pool: &sqlx::PgPool,
    gateway_id: uuid::Uuid,
    old_instance_id: uuid::Uuid,
    old_revision_id: uuid::Uuid,
    candidate_revision_id: uuid::Uuid,
) -> uuid::Uuid {
    tokio::time::timeout(Duration::from_secs(18), async {
        loop {
            let row: Option<(Option<uuid::Uuid>, String, uuid::Uuid, String)> = sqlx::query_as(
                "SELECT gateway.active_revision_id, old_instance.state,
                        candidate.id, candidate.state
                   FROM gateways AS gateway
                   JOIN gateway_service_instances AS old_instance
                     ON old_instance.id = $2
                    AND old_instance.gateway_id = gateway.id
                    AND old_instance.revision_id = $3
                   JOIN gateway_service_instances AS candidate
                     ON candidate.gateway_id = gateway.id
                    AND candidate.revision_id = $4
                  WHERE gateway.id = $1
                    AND candidate.state = 'ready'
                  ORDER BY candidate.created_at DESC
                  LIMIT 1",
            )
            .bind(gateway_id)
            .bind(old_instance_id)
            .bind(old_revision_id)
            .bind(candidate_revision_id)
            .fetch_optional(pool)
            .await
            .expect("read coherent service cutover state");
            if let Some((Some(active), old_state, candidate_id, candidate_state)) = row {
                if active == candidate_revision_id
                    && old_state == "draining"
                    && candidate_state == "ready"
                {
                    return candidate_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("B becomes active while A drains")
}

pub async fn exercise_gateway_service_cutover_requests(
    pool: &sqlx::PgPool,
    candidate: &GatewayServiceGoldenFixture,
    candidate_instance_id: uuid::Uuid,
    public_url: &str,
    started_at: OffsetDateTime,
) -> GatewayServiceRequestProof {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("bounded B cutover client");
    let first_nonce = uuid::Uuid::new_v4();
    let second_nonce = uuid::Uuid::new_v4();
    let path = format!("{public_url}/gateway/service/identity");
    let first = client
        .get(format!("{path}?cutover_nonce={first_nonce}"))
        .send()
        .await
        .expect("first B public identity request")
        .error_for_status()
        .expect("first B identity succeeds")
        .bytes()
        .await
        .expect("read first B identity");
    let second = client
        .get(format!("{path}?cutover_nonce={second_nonce}"))
        .send()
        .await
        .expect("second B public identity request")
        .error_for_status()
        .expect("second B identity succeeds")
        .bytes()
        .await
        .expect("read second B identity");
    let first: serde_json::Value = serde_json::from_slice(&first).expect("first B identity JSON");
    let second: serde_json::Value =
        serde_json::from_slice(&second).expect("second B identity JSON");
    let first_startup = first
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .expect("first B startup identity");
    let second_startup = second
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .expect("second B startup identity");
    assert_eq!(first_startup, second_startup);
    assert!(
        second
            .get("request_count")
            .and_then(serde_json::Value::as_u64)
            .expect("second B request count")
            > first
                .get("request_count")
                .and_then(serde_json::Value::as_u64)
                .expect("first B request count")
    );
    // The nonce values make the two public URLs distinct.  Invocation rows do
    // not persist query strings, so the DB correlation is the exact two-row
    // delta after the exclusive cutover request window.
    let candidate_fence: i64 =
        sqlx::query_scalar("SELECT fencing_token FROM gateway_service_instances WHERE id = $1")
            .bind(candidate_instance_id)
            .fetch_one(pool)
            .await
            .expect("read B fencing token");
    let bindings: Vec<(uuid::Uuid, Option<uuid::Uuid>, Option<i64>, String)> = sqlx::query_as(
        "SELECT gateway_revision_id, service_instance_id,
                service_instance_fencing_token, outcome
           FROM gateway_invocations
          WHERE gateway_id = $1 AND accepted_at >= $2
          ORDER BY accepted_at DESC, id DESC
          LIMIT 3",
    )
    .bind(candidate.gateway_id)
    .bind(started_at)
    .fetch_all(pool)
    .await
    .expect("read B invocation bindings");
    assert_eq!(bindings.len(), 2);
    assert!(
        bindings
            .iter()
            .all(|(revision, instance, fencing_token, outcome)| {
                *revision == candidate.revision_id
                    && *instance == Some(candidate_instance_id)
                    && *fencing_token == Some(candidate_fence)
                    && outcome == "completed"
            },)
    );
    GatewayServiceRequestProof {
        pid: first
            .get("pid")
            .and_then(serde_json::Value::as_u64)
            .expect("B guest PID"),
        startup_id: first_startup.to_owned(),
    }
}
