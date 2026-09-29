use crate::support::{LifecycleState, SENTINEL, identity, key, pool};
use authz_postgres::PostgresMelangeAuthorizer;
use secret_application::{CreateSecret, SecretServiceError};
use secret_domain::{
    DeliveryMode, SecretId, SecretName, SecretOwner, SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use std::sync::Arc;

pub(super) async fn initialize() -> Option<LifecycleState> {
    let pool = pool().await?;
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");
    let fixture = crate::support::seed(&pool).await;
    let service = Arc::new(SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new(
                "test/v1",
                [("test/v1", [7_u8; 32]), ("test/v2", [8_u8; 32])],
            )
            .expect("fixture keys should validate"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    ));
    let owner = identity(fixture.owner);
    let target_manager = identity(fixture.target_manager);
    let organization_secret_manager = identity(fixture.organization_secret_manager);
    let ordinary_member = identity(fixture.ordinary_member);
    let outsider = identity(fixture.other_owner);
    Some(LifecycleState {
        pool,
        fixture,
        service,
        owner,
        target_manager,
        organization_secret_manager,
        ordinary_member,
        outsider,
        secret_id: SecretId::new(),
        first_version: SecretVersionId::new(),
        import_id: None,
        repository_import_id: None,
        repository_grant_id: None,
        instance_id: None,
        initial_revision_id: None,
        attachment_id: None,
        second_attachment_id: None,
        bound_revision_id: None,
        repository_bound_revision_id: None,
        run_id: None,
        brokered_binding_id: None,
        brokered_rule_id: None,
        authority: None,
        raw_revision_id: None,
        raw_run_id: None,
        raw_authority: None,
        update_revision_id: None,
        update_run_id: None,
        update_authority: None,
        pinned_run: None,
        pinned_authority: None,
        revoked_pinned_run: None,
        revoked_pinned_authority: None,
        pre_revoked_run: None,
        pre_revoked_authority: None,
        active_version: None,
        expected_rotated_value: None,
    })
}

// This phase keeps its ordered database assertions together to preserve the lifecycle scenario.
#[allow(clippy::too_many_lines)]
pub(super) async fn create_secrets(state: &mut LifecycleState) {
    let pool = state.pool.clone();
    let fixture = &state.fixture;
    let service = Arc::clone(&state.service);
    let owner = state.owner.clone();
    let organization_secret_manager = state.organization_secret_manager.clone();
    let ordinary_member = state.ordinary_member.clone();
    let outsider = state.outsider.clone();
    let manager_secret_id = SecretId::new();

    service
        .create(
            &organization_secret_manager,
            CreateSecret {
                command_key: key("manager-create", manager_secret_id.as_uuid()),
                secret_id: manager_secret_id,
                version_id: SecretVersionId::new(),
                owner: SecretOwner::Organization(fixture.organization),
                name: SecretName::parse(format!("manager-{manager_secret_id}").replace('-', "_"))
                    .expect("manager secret name"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered],
                value: SecretValue::new("manager-only-value").expect("manager secret value"),
            },
        )
        .await
        .expect("explicit organization secret manager should create");
    for (identity, label) in [
        (&ordinary_member, "ordinary-member"),
        (&outsider, "cross-tenant-outsider"),
    ] {
        let denied_secret_id = SecretId::new();
        let denied = service
            .create(
                identity,
                CreateSecret {
                    command_key: key(label, denied_secret_id.as_uuid()),
                    secret_id: denied_secret_id,
                    version_id: SecretVersionId::new(),
                    owner: SecretOwner::Organization(fixture.organization),
                    name: SecretName::parse(format!("denied-{denied_secret_id}").replace('-', "_"))
                        .expect("denied secret name"),
                    allowed_delivery_modes: vec![DeliveryMode::Brokered],
                    value: SecretValue::new("must-not-persist").expect("denied secret value"),
                },
            )
            .await;
        assert!(matches!(
            denied,
            Err(SecretServiceError::AuthorizationDenied)
        ));
    }
    let secret_id = state.secret_id;
    let first_version = state.first_version;
    let create_key = key("create", secret_id.as_uuid());
    let created = service
        .create(
            &owner,
            CreateSecret {
                command_key: create_key,
                secret_id,
                version_id: first_version,
                owner: SecretOwner::Organization(fixture.organization),
                name: SecretName::parse(format!("model-{secret_id}").replace('-', "_"))
                    .expect("fixture name should validate"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered, DeliveryMode::Raw],
                value: SecretValue::new(SENTINEL).expect("sentinel should validate"),
            },
        )
        .await
        .expect("organization owner should create");
    assert_eq!(created.secret_id, secret_id);
    assert_eq!(created.version_id, first_version);

    let duplicate = service
        .create(
            &owner,
            CreateSecret {
                command_key: create_key,
                secret_id,
                version_id: first_version,
                owner: SecretOwner::Organization(fixture.organization),
                name: SecretName::parse(format!("model-{secret_id}").replace('-', "_"))
                    .expect("fixture name should validate"),
                allowed_delivery_modes: vec![DeliveryMode::Raw, DeliveryMode::Brokered],
                value: SecretValue::new(SENTINEL).expect("sentinel should validate"),
            },
        )
        .await
        .expect("duplicate create should return durable result");
    assert_eq!(duplicate, created);

    let stored: (Vec<u8>, Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT ciphertext, wrapped_data_key, associated_data_hash
          FROM secret_versions WHERE id = $1",
    )
    .bind(first_version.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("stored encrypted version");
    for bytes in [&stored.0, &stored.1, &stored.2] {
        assert!(
            !bytes
                .windows(SENTINEL.len())
                .any(|value| value == SENTINEL.as_bytes())
        );
    }
    let durable_text: String = sqlx::query_scalar(
        "SELECT concat_ws(' ',
             (SELECT string_agg(row_to_json(secrets)::text, ' ') FROM secrets
              WHERE id = $1),
             (SELECT string_agg(payload::text, ' ') FROM outbox
              WHERE aggregate_id = $1),
             (SELECT string_agg(row_to_json(secret_audit_events)::text, ' ')
              FROM secret_audit_events WHERE secret_id = $1))",
    )
    .bind(secret_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("inspect non-ciphertext durable records");
    assert!(!durable_text.contains(SENTINEL));
    state.secret_id = secret_id;
    state.first_version = first_version;
}
