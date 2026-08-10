//! Opt-in `PostgreSQL` coverage for developer PAT persistence and lifecycle.

use forge_domain::RepositoryId;
use git_capability_domain::GitOperation;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use pat_domain::{PersonalAccessTokenLabel, PersonalAccessTokenScope};
use pat_postgres::{
    CreatePersonalAccessToken, PersonalAccessTokenServiceError, PostgresPersonalAccessTokenService,
    RotatePersonalAccessToken,
};
use serde_json::json;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use time::{Duration, OffsetDateTime};

#[tokio::test]
#[serial]
async fn persists_hash_only_tokens_and_enforces_lifecycle_and_scope() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let identity = create_user(&pool, "PAT lifecycle owner").await;
    let service = PostgresPersonalAccessTokenService::new(pool.clone());
    let allowed_repository = RepositoryId::new();
    let denied_repository = RepositoryId::new();
    let scope = PersonalAccessTokenScope::new(
        [GitOperation::Discover, GitOperation::Fetch],
        Some([allowed_repository]),
    )
    .expect("valid exact scope");
    let issued = service
        .create(
            &identity,
            CreatePersonalAccessToken {
                label: PersonalAccessTokenLabel::parse("developer laptop").expect("valid label"),
                scope,
                expires_at: OffsetDateTime::now_utc() + Duration::hours(1),
            },
        )
        .await
        .expect("issue PAT");
    let plaintext = issued.token.expose();
    let stored: (Vec<u8>, Vec<String>, Option<Vec<uuid::Uuid>>) = sqlx::query_as(
        "SELECT verifier_digest, git_operations, repository_restrictions
         FROM developer_personal_access_tokens WHERE id = $1",
    )
    .bind(issued.metadata.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("stored PAT verifier");
    assert_eq!(stored.0.len(), 32);
    assert!(
        !plaintext
            .as_bytes()
            .windows(32)
            .any(|bytes| bytes == stored.0)
    );
    assert_eq!(stored.1, vec!["discover", "fetch"]);
    assert_eq!(stored.2, Some(vec![allowed_repository.as_uuid()]));

    let listed = service.list(&identity).await.expect("safe PAT listing");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0], issued.metadata);

    let authenticated = service
        .authenticate(
            &issued.token,
            GitOperation::Fetch,
            allowed_repository,
            RequestId::new(),
        )
        .await
        .expect("authenticate allowed exact scope");
    assert_eq!(authenticated.owner_user_id, identity.user_id);
    let last_used_at: Option<OffsetDateTime> = sqlx::query_scalar(
        "SELECT last_used_at FROM developer_personal_access_tokens WHERE id = $1",
    )
    .bind(issued.metadata.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("last-used metadata");
    assert!(last_used_at.is_some());
    assert_eq!(
        service
            .authenticate(
                &issued.token,
                GitOperation::Fetch,
                denied_repository,
                RequestId::new(),
            )
            .await,
        Err(PersonalAccessTokenServiceError::InvalidCredential)
    );

    let replacement = service
        .rotate(
            &identity,
            RotatePersonalAccessToken {
                token_id: issued.metadata.id,
                label: PersonalAccessTokenLabel::parse("replacement").expect("valid label"),
                scope: PersonalAccessTokenScope::new(
                    [GitOperation::Receive],
                    Some([allowed_repository]),
                )
                .expect("replacement scope"),
                expires_at: OffsetDateTime::now_utc() + Duration::hours(1),
            },
        )
        .await
        .expect("rotate PAT atomically");
    assert_eq!(
        service
            .authenticate(
                &issued.token,
                GitOperation::Fetch,
                allowed_repository,
                RequestId::new(),
            )
            .await,
        Err(PersonalAccessTokenServiceError::InvalidCredential)
    );
    service
        .authenticate(
            &replacement.token,
            GitOperation::Receive,
            allowed_repository,
            RequestId::new(),
        )
        .await
        .expect("replacement authenticates");
    service
        .revoke(&identity, replacement.metadata.id)
        .await
        .expect("revoke replacement");
    assert_eq!(
        service
            .authenticate(
                &replacement.token,
                GitOperation::Receive,
                allowed_repository,
                RequestId::new(),
            )
            .await,
        Err(PersonalAccessTokenServiceError::InvalidCredential)
    );

    let audit_types: Vec<String> = sqlx::query_scalar(
        "SELECT event_type FROM personal_access_token_audit_events
         WHERE owner_user_id = $1 ORDER BY occurred_at, id",
    )
    .bind(identity.user_id.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("PAT audit trail");
    assert!(audit_types.iter().any(|event| event == "issued"));
    assert_eq!(
        audit_types
            .iter()
            .filter(|event| event.as_str() == "rotated")
            .count(),
        2
    );
    assert!(audit_types.iter().any(|event| event == "revoked"));
    assert_eq!(
        audit_types
            .iter()
            .filter(|event| event.as_str() == "authenticated")
            .count(),
        2
    );
}

#[tokio::test]
#[serial]
async fn authentication_rejects_a_pat_owned_by_an_inactive_user() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let identity = create_user(&pool, "Suspended PAT owner").await;
    let service = PostgresPersonalAccessTokenService::new(pool.clone());
    let repository = RepositoryId::new();
    let issued = service
        .create(
            &identity,
            CreatePersonalAccessToken {
                label: PersonalAccessTokenLabel::parse("suspension fixture").expect("valid label"),
                scope: PersonalAccessTokenScope::new([GitOperation::Fetch], Some([repository]))
                    .expect("valid scope"),
                expires_at: OffsetDateTime::now_utc() + Duration::hours(1),
            },
        )
        .await
        .expect("issue PAT");
    sqlx::query("UPDATE users SET status = 'suspended' WHERE id = $1")
        .bind(identity.user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("suspend PAT owner");

    assert_eq!(
        service
            .authenticate(
                &issued.token,
                GitOperation::Fetch,
                repository,
                RequestId::new(),
            )
            .await,
        Err(PersonalAccessTokenServiceError::InvalidCredential)
    );
}

#[tokio::test]
#[serial]
async fn application_role_rls_exposes_only_the_actor_tokens() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let first = create_user(&pool, "First PAT RLS owner").await;
    let second = create_user(&pool, "Second PAT RLS owner").await;
    let service = PostgresPersonalAccessTokenService::new(pool.clone());
    for identity in [&first, &second] {
        service
            .create(
                identity,
                CreatePersonalAccessToken {
                    label: PersonalAccessTokenLabel::parse("RLS token").expect("valid label"),
                    scope: PersonalAccessTokenScope::new(
                        [GitOperation::Discover],
                        None::<[RepositoryId; 0]>,
                    )
                    .expect("unrestricted repository scope"),
                    expires_at: OffsetDateTime::now_utc() + Duration::hours(1),
                },
            )
            .await
            .expect("issue RLS fixture");
    }

    let mut transaction = pool.begin().await.expect("begin RLS transaction");
    sqlx::query("SET LOCAL ROLE hephaestus_app")
        .execute(&mut *transaction)
        .await
        .expect("assume application role");
    sqlx::query("SELECT set_config('hephaestus.actor_id', $1, true)")
        .bind(first.user_id.to_string())
        .execute(&mut *transaction)
        .await
        .expect("set actor context");
    let visible_owners: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT owner_user_id FROM developer_personal_access_tokens")
            .fetch_all(&mut *transaction)
            .await
            .expect("RLS-protected token listing");
    assert!(!visible_owners.is_empty());
    assert!(
        visible_owners
            .iter()
            .all(|owner| *owner == first.user_id.as_uuid())
    );
}

async fn test_pool() -> Option<sqlx::PgPool> {
    let database_url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("connect to PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply PAT migrations");
    Some(pool)
}

async fn create_user(pool: &sqlx::PgPool, display_name: &str) -> AuthenticatedIdentity {
    let user_id = UserId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(user_id.as_uuid())
        .bind(display_name)
        .execute(pool)
        .await
        .expect("PAT user fixture");
    AuthenticatedIdentity::new(
        user_id,
        "https://issuer.example",
        format!("pat-user-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}
