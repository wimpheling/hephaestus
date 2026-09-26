use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

#[derive(Clone, Copy)]
pub struct Fixture {
    pub owner: UserId,
    pub admin: UserId,
    pub maintainer: UserId,
    pub member: UserId,
    pub outsider: UserId,
    pub revoked: UserId,
    pub organization: Uuid,
    pub project: Uuid,
    pub consuming_project: Uuid,
    pub private_repository: Uuid,
    pub public_repository: Uuid,
    pub consuming_repository: Uuid,
    pub build: Uuid,
    pub release: Uuid,
    pub artifact: Uuid,
    pub release_agent: Uuid,
    pub instance: Uuid,
    pub attachment: Uuid,
    pub update: Uuid,
    pub run: Uuid,
    pub volume: Uuid,
}

pub async fn pool() -> Option<PgPool> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    Some(
        PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .expect("connect to PostgreSQL"),
    )
}

pub fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://issuer.example",
        format!("subject-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}
