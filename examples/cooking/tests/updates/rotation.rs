// Reuse the update facade imports across focused lifecycle phases.
#[allow(unused_imports)]
use super::*;
pub(crate) async fn wait_for_active_brokered_lease(
    pool: &PgPool,
    run_id: Uuid,
    rule_id: Uuid,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*)
               FROM brokered_secret_lease_snapshots snapshot
               JOIN secret_leases lease ON lease.id = snapshot.lease_id
              WHERE snapshot.run_id = $1 AND snapshot.rule_id = $2
                AND lease.status = 'active' AND lease.expires_at > now()",
        )
        .bind(run_id)
        .bind(rule_id)
        .fetch_one(pool)
        .await
        .expect("active brokered lease before rotation");
        if count == 1 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "held v1 relay lease was not issued before rotation"
        );
        sleep(Duration::from_millis(25)).await;
    }
}

/// Rotates the model secret after the held v1 run has acquired its lease.
/// The operation uses the same authenticated secret service boundary as the
/// product RPC and returns only opaque version identities.
pub(crate) async fn rotate_model_credential_while_held(
    context: &CookingUpdateContext<'_>,
    held_run_id: Uuid,
) -> CredentialRotation {
    let (secret_id, pinned_version_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT imported.secret_id, snapshot.secret_version_id
           FROM brokered_secret_lease_snapshots snapshot
           JOIN brokered_secret_rules rule ON rule.id = snapshot.rule_id
           JOIN agent_secret_bindings binding ON binding.id = snapshot.binding_id
           JOIN secret_imports imported ON imported.id = binding.import_id
          WHERE snapshot.run_id = $1 AND rule.id = $2",
    )
    .bind(held_run_id)
    .bind(super::super::cooking::MODEL_RULE)
    .fetch_one(context.pool)
    .await
    .expect("held v1 model lease snapshot");
    let owner = UserId::from_uuid(context.owner);
    let identity = AuthenticatedIdentity::new(
        owner,
        crate::golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let new_version = SecretVersionId::new();
    let service = SecretService::new(
        context.pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("cooking rotation key"),
        ),
        std::sync::Arc::new(PostgresMelangeAuthorizer),
    );
    service
        .rotate(
            &identity,
            RotateSecret {
                command_key: SecretCommandKey::derive(
                    "cooking-rotate-model",
                    &[new_version.as_uuid().as_bytes()],
                ),
                secret_id: SecretId::from_uuid(secret_id),
                expected_active_version_id: SecretVersionId::from_uuid(pinned_version_id),
                new_version_id: new_version,
                value: SecretValue::new(super::super::cooking::MODEL_ROTATED_SENTINEL)
                    .expect("rotated model sentinel"),
            },
        )
        .await
        .expect("rotate model credential while v1 lease is held");
    let active_version: Uuid =
        sqlx::query_scalar("SELECT active_version_id FROM secrets WHERE id = $1")
            .bind(secret_id)
            .fetch_one(context.pool)
            .await
            .expect("rotated model active version");
    assert_eq!(active_version, new_version.as_uuid());
    CredentialRotation {
        pinned_version_id,
        rotated_version_id: active_version,
    }
}

pub(crate) async fn assert_rotated_model_lease(
    pool: &PgPool,
    held_run_id: Uuid,
    deferred_event_id: Uuid,
    current_model_rule_id: Uuid,
    rotation: CredentialRotation,
) {
    let held_version: Uuid = sqlx::query_scalar(
        "SELECT secret_version_id FROM brokered_secret_lease_snapshots
          WHERE run_id = $1 AND rule_id = $2",
    )
    .bind(held_run_id)
    .bind(super::super::cooking::MODEL_RULE)
    .fetch_one(pool)
    .await
    .expect("held model lease version");
    assert_eq!(held_version, rotation.pinned_version_id);
    let later_version: Uuid = sqlx::query_scalar(
        "SELECT snapshot.secret_version_id
           FROM mailbox_delivery_attempts attempt
           JOIN brokered_secret_lease_snapshots snapshot
             ON snapshot.run_id = attempt.run_id AND snapshot.rule_id = $2
          WHERE attempt.event_id = $1
          ORDER BY attempt.attempt_number DESC
          LIMIT 1",
    )
    .bind(deferred_event_id)
    .bind(current_model_rule_id)
    .fetch_one(pool)
    .await
    .expect("later model lease version");
    assert_eq!(later_version, rotation.rotated_version_id);
    assert_ne!(held_version, later_version);
}
