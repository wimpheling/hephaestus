//! Opt-in `PostgreSQL` coverage for verified identity mapping.

use identity_application::{VerifiedExternalIdentity, VerifiedIdentityMapper};
use identity_domain::{RequestId, UserId};
use identity_postgres::PostgresIdentityStore;
use serde_json::json;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;

#[tokio::test]
#[serial]
async fn maps_exact_issuer_subject_and_rejects_inactive_users() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("connect to PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("Phase 3 migrations");
    let user_id = UserId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'OIDC user')")
        .bind(user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("user fixture");
    sqlx::query(
        "INSERT INTO external_identities
         (user_id, issuer, subject, provider_metadata)
         VALUES ($1, 'https://issuer.example', 'subject', '{}')",
    )
    .bind(user_id.as_uuid())
    .execute(&pool)
    .await
    .expect("identity fixture");
    let verified = VerifiedExternalIdentity {
        issuer: String::from("https://issuer.example"),
        subject: String::from("subject"),
        claims: json!({"email": "user@example.invalid", "email_verified": true}),
    };
    let store = PostgresIdentityStore::new(pool.clone());
    let mapped = store
        .map_verified_identity(&verified, RequestId::new(), Some("trace"))
        .await
        .expect("mapped identity");
    assert_eq!(mapped.user_id, user_id);
    assert_eq!(mapped.trace_id.as_deref(), Some("trace"));
    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("suspend user");
    assert!(matches!(
        store
            .map_verified_identity(&verified, RequestId::new(), None)
            .await,
        Err(identity_application::IdentityMappingError::Inactive)
    ));
}
