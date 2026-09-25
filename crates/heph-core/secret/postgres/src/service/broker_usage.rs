use super::*;

pub(super) async fn record_runtime_use(
    resolver_pool: &PgPool,
    session: &RuntimeSessionRow,
    lease: &RuntimeLeaseAuthorizationRow,
    outcome: &str,
) -> Result<(), SecretServiceError> {
    let mut tx = resolver_pool
        .begin()
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    record_runtime_use_tx(&mut tx, session, lease, DeliveryMode::Brokered, outcome)
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    tx.commit()
        .await
        .map_err(|_| SecretServiceError::Persistence)?;
    Ok(())
}

// The decision commits before the external call, and its outcome commits after.
// A decision without an outcome honestly represents an interrupted operation.
#[allow(clippy::too_many_arguments)] // Keep the value-free audit fields explicit at this boundary.
pub(super) async fn record_https_operation(
    pool: &PgPool,
    session: &RuntimeSessionRow,
    lease: &RuntimeLeaseAuthorizationRow,
    request_id: Uuid,
    rule_id: Option<Uuid>,
    decision: Option<&str>,
    outcome: Option<&str>,
    reason_code: Option<&str>,
) -> Result<(), SecretServiceError> {
    let inserted = sqlx::query(
        "INSERT INTO brokered_secret_audit_events
         (id, lease_snapshot_id, rule_id, runtime_session_id, run_id,
          request_id, event_kind, decision, outcome, reason_code, occurred_at)
         SELECT $1, snapshot.id, snapshot.rule_id, snapshot.runtime_session_id,
                snapshot.run_id, $2, $3, $4, $5, $6, now()
         FROM brokered_secret_lease_snapshots snapshot
         WHERE snapshot.lease_id = $7 AND snapshot.runtime_session_id = $8
           AND snapshot.run_id = $9 AND snapshot.rule_id = $10",
    )
    .bind(Uuid::new_v4())
    .bind(request_id)
    .bind(if decision.is_some() {
        "authorization_decision"
    } else {
        "substitution_use"
    })
    .bind(decision)
    .bind(outcome)
    .bind(reason_code)
    .bind(lease.lease_id)
    .bind(session.session_id)
    .bind(session.run_id)
    .bind(rule_id)
    .execute(pool)
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
    if (decision == Some("allow") || outcome.is_some()) && inserted.rows_affected() != 1 {
        return Err(SecretServiceError::Persistence);
    }
    Ok(())
}

pub(super) async fn record_runtime_use_tx(
    tx: &mut Transaction<'_, Postgres>,
    session: &RuntimeSessionRow,
    lease: &RuntimeLeaseAuthorizationRow,
    mode: DeliveryMode,
    outcome: &str,
) -> Result<(), SecretServiceError> {
    let inserted = sqlx::query(
        "INSERT INTO secret_audit_events
           (id, owner_organization_id, runtime_run_id, secret_id,
            secret_version_id, grant_id, import_id, binding_id, lease_id,
            operation, permission, delivery_mode, decision, outcome,
            authorization_model_version, policy_version)
           SELECT $1, secret.owner_organization_id, $2, provenance.secret_id,
                  provenance.secret_version_id, provenance.grant_id,
                  provenance.import_id, provenance.binding_id, lease.id,
                  $3, $4, $5, 'allow', $6, $7, 'runtime/v1'
           FROM secret_leases AS lease
           JOIN run_secret_provenance AS provenance
             ON provenance.run_id = lease.run_id
            AND provenance.binding_id = lease.binding_id
           JOIN secrets AS secret ON secret.id = provenance.secret_id
           WHERE lease.id = $8 AND lease.session_id = $9
             AND lease.run_id = $2",
    )
    .bind(Uuid::new_v4())
    .bind(session.run_id)
    .bind(if mode == DeliveryMode::Raw {
        "receive_raw"
    } else {
        "use_brokered"
    })
    .bind(if mode == DeliveryMode::Raw {
        "secret.receive_raw"
    } else {
        "secret.use_brokered"
    })
    .bind(mode_name(mode))
    .bind(outcome)
    .bind(AUTHORIZATION_MODEL_VERSION)
    .bind(lease.lease_id)
    .bind(session.session_id)
    .execute(&mut **tx)
    .await
    .map_err(|_| SecretServiceError::Persistence)?;
    if inserted.rows_affected() != 1 {
        return Err(SecretServiceError::Unavailable);
    }
    Ok(())
}

pub(super) fn validate_broker_request(request: &BrokerRequest) -> Result<(), SecretServiceError> {
    let valid_operation = (1..=64).contains(&request.operation.len())
        && request
            .operation
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_' || byte.is_ascii_digit());
    let valid_destination = (1..=253).contains(&request.destination.len())
        && request.destination.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
        })
        && request.destination.parse::<std::net::IpAddr>().is_err()
        && request.destination.rsplit('.').next() != Some("local")
        && request.destination != "localhost";
    if !valid_operation || !valid_destination || request.body.len() > 65_536 {
        return Err(SecretServiceError::BrokerRequestDenied);
    }
    Ok(())
}

pub(super) fn broker_request_rule_id(request: &BrokerRequest) -> Option<Uuid> {
    serde_json::from_slice::<serde_json::Value>(&request.body)
        .ok()
        .and_then(|value| {
            value
                .get("rule_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok())
        })
}
