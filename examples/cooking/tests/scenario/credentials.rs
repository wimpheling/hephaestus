// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
/// Replacement model credential used only after the held v1 lease is pinned.
pub const MODEL_ROTATED_SENTINEL: &str = "cooking-model-rotated-fixture-sentinel-936f";
/// Replacement inbound credential used by the next immutable gateway revision.
pub const INBOUND_ROTATED_SENTINEL: &str = "cooking-inbound-rotated-fixture-sentinel-157a";
/// Replacement relay credential selected after the old lease has been pinned.
pub const RELAY_ROTATED_SENTINEL: &str = "cooking-relay-rotated-fixture-sentinel-482b";
/// All fixture credential sentinels that must remain outside persisted or
/// externally visible evidence.
pub const FIXTURE_CREDENTIAL_SENTINELS: &[&str] = &[
    INBOUND_SENTINEL,
    MODEL_SENTINEL,
    RELAY_SENTINEL,
    MODEL_ROTATED_SENTINEL,
    INBOUND_ROTATED_SENTINEL,
    RELAY_ROTATED_SENTINEL,
];

pub(crate) fn assert_no_credentials(value: &str) {
    for sentinel in FIXTURE_CREDENTIAL_SENTINELS {
        assert!(!value.contains(sentinel), "fixture credential exposed");
    }
}

/// Opaque version identities returned by a supported secret rotation.  The
/// values are intentionally kept separate from plaintext credentials so the
/// caller can prove lease pinning without putting secret material in logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CredentialRotation {
    pub pinned_version_id: uuid::Uuid,
    pub rotated_version_id: uuid::Uuid,
}

/// Rotates the source secret used by one brokered rule after an already
/// admitted run has acquired its lease.  This uses the same authenticated
/// `SecretService` boundary as the application and leaves the old immutable
/// version available for the historical lease query.
pub(crate) async fn rotate_brokered_credential(
    pool: &sqlx::PgPool,
    actor: UserId,
    held_run_id: uuid::Uuid,
    rule_id: uuid::Uuid,
    replacement: &str,
) -> CredentialRotation {
    let (secret_id, pinned_version_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT imported.secret_id, snapshot.secret_version_id
           FROM brokered_secret_lease_snapshots snapshot
           JOIN brokered_secret_rules rule ON rule.id = snapshot.rule_id
           JOIN agent_secret_bindings binding ON binding.id = snapshot.binding_id
           JOIN secret_imports imported ON imported.id = binding.import_id
          WHERE snapshot.run_id = $1 AND rule.id = $2",
    )
    .bind(held_run_id)
    .bind(rule_id)
    .fetch_one(pool)
    .await
    .expect("brokered lease source secret");
    let identity = AuthenticatedIdentity::new(
        actor,
        crate::golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("cooking rotation key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let rotated_version_id = SecretVersionId::new();
    service
        .rotate(
            &identity,
            RotateSecret {
                command_key: secret_command_key("cooking-rotate-brokered", uuid::Uuid::new_v4()),
                secret_id: SecretId::from_uuid(secret_id),
                expected_active_version_id: SecretVersionId::from_uuid(pinned_version_id),
                new_version_id: rotated_version_id,
                value: SecretValue::new(replacement).expect("replacement sentinel"),
            },
        )
        .await
        .expect("rotate brokered cooking credential");
    let active_version: uuid::Uuid =
        sqlx::query_scalar("SELECT active_version_id FROM secrets WHERE id = $1")
            .bind(secret_id)
            .fetch_one(pool)
            .await
            .expect("rotated brokered active version");
    assert_eq!(active_version, rotated_version_id.as_uuid());
    CredentialRotation {
        pinned_version_id,
        rotated_version_id: active_version,
    }
}

/// Returns the project import selected by a durable brokered rule. This keeps
/// callers on the persisted rule/binding graph when they subsequently revoke
/// the source through `SecretService`.
pub(crate) async fn import_id_for_brokered_rule(
    pool: &sqlx::PgPool,
    rule_id: uuid::Uuid,
) -> uuid::Uuid {
    sqlx::query_scalar(
        "SELECT imported.id
           FROM brokered_secret_rules rule
           JOIN agent_secret_bindings binding ON binding.id = rule.binding_id
           JOIN secret_imports imported ON imported.id = binding.import_id
          WHERE rule.id = $1",
    )
    .bind(rule_id)
    .fetch_one(pool)
    .await
    .expect("brokered rule import")
}

/// Rotates the inbound secret selected by a gateway revision.  Gateway
/// reconfiguration is deliberately a separate call: an immutable revision
/// must retain the old selected version while the caller configures a new
/// revision through the normal Gateway RPC.
pub(crate) async fn rotate_inbound_credential(
    pool: &sqlx::PgPool,
    actor: UserId,
    import_id: uuid::Uuid,
    expected_version_id: uuid::Uuid,
) -> CredentialRotation {
    let (secret_id, active_version_id): (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT secret_id, active_version_id FROM secret_imports
          JOIN secrets ON secrets.id = secret_imports.secret_id
         WHERE secret_imports.id = $1",
    )
    .bind(import_id)
    .fetch_one(pool)
    .await
    .expect("inbound source secret");
    assert_eq!(active_version_id, expected_version_id);
    rotate_secret_value(
        pool,
        actor,
        secret_id,
        expected_version_id,
        INBOUND_ROTATED_SENTINEL,
        "cooking-rotate-inbound",
    )
    .await
}

pub(crate) async fn rotate_secret_value(
    pool: &sqlx::PgPool,
    actor: UserId,
    secret_id: uuid::Uuid,
    pinned_version_id: uuid::Uuid,
    replacement: &str,
    operation: &str,
) -> CredentialRotation {
    let identity = AuthenticatedIdentity::new(
        actor,
        crate::golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("inbound rotation key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let rotated_version_id = SecretVersionId::new();
    service
        .rotate(
            &identity,
            RotateSecret {
                command_key: secret_command_key(operation, uuid::Uuid::new_v4()),
                secret_id: SecretId::from_uuid(secret_id),
                expected_active_version_id: SecretVersionId::from_uuid(pinned_version_id),
                new_version_id: rotated_version_id,
                value: SecretValue::new(replacement).expect("replacement sentinel"),
            },
        )
        .await
        .expect("rotate cooking credential");
    CredentialRotation {
        pinned_version_id,
        rotated_version_id: rotated_version_id.as_uuid(),
    }
}

/// Revokes an imported cooking source through the authenticated secret service
/// and verifies the source/import/binding lifecycle was durably fenced.
pub(crate) async fn revoke_imported_credential(
    pool: &sqlx::PgPool,
    actor: UserId,
    import_id: uuid::Uuid,
) {
    let secret_id: uuid::Uuid =
        sqlx::query_scalar("SELECT secret_id FROM secret_imports WHERE id = $1")
            .bind(import_id)
            .fetch_one(pool)
            .await
            .expect("credential import source");
    let identity = AuthenticatedIdentity::new(
        actor,
        crate::golden_issuer(),
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("credential revoke key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    service
        .revoke_secret(
            &identity,
            secret_command_key("cooking-revoke-imported", import_id),
            SecretId::from_uuid(secret_id),
        )
        .await
        .expect("revoke cooking credential");
    let states: (String, String) = sqlx::query_as(
        "SELECT secrets.status, secret_imports.status
           FROM secrets JOIN secret_imports ON secret_imports.secret_id = secrets.id
          WHERE secret_imports.id = $1",
    )
    .bind(import_id)
    .fetch_one(pool)
    .await
    .expect("revoked credential state");
    assert_eq!(states, (String::from("revoked"), String::from("revoked")));
}

/// Compares immutable lease snapshots for an old and a later dispatch of the
/// same rule. This is the durable proof that rotation changes future leases
/// without rewriting the in-flight run's selected version.
pub(crate) async fn assert_rotated_brokered_lease(
    pool: &sqlx::PgPool,
    held_run_id: uuid::Uuid,
    later_event_id: uuid::Uuid,
    held_rule_id: uuid::Uuid,
    later_rule_id: uuid::Uuid,
    rotation: CredentialRotation,
) {
    let held: uuid::Uuid = sqlx::query_scalar(
        "SELECT secret_version_id FROM brokered_secret_lease_snapshots
          WHERE run_id = $1 AND rule_id = $2",
    )
    .bind(held_run_id)
    .bind(held_rule_id)
    .fetch_one(pool)
    .await
    .expect("held brokered lease version");
    let later: uuid::Uuid = sqlx::query_scalar(
        "SELECT snapshot.secret_version_id
           FROM mailbox_delivery_attempts attempt
           JOIN brokered_secret_lease_snapshots snapshot
             ON snapshot.run_id = attempt.run_id AND snapshot.rule_id = $2
           WHERE attempt.event_id = $1
          ORDER BY attempt.attempt_number DESC LIMIT 1",
    )
    .bind(later_event_id)
    .bind(later_rule_id)
    .fetch_one(pool)
    .await
    .expect("later brokered lease version");
    assert_eq!(held, rotation.pinned_version_id);
    assert_eq!(later, rotation.rotated_version_id);
    assert_ne!(held, later);
}

pub(crate) struct CookingAdapters(
    pub  Arc<
        Mutex<std::collections::HashMap<uuid::Uuid, Arc<dyn secret_application::BrokerAdapter>>>,
    >,
);
