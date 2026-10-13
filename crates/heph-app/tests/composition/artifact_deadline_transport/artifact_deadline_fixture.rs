//! Existing token and relational artifact fixture, unchanged transport evidence.
use super::stream_support::now_seconds;
use identity_domain::BrowserSessionSid;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rpc_proto::messages::hephaestus::common::v1::OpaqueId;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{fs, path::Path};
use uuid::Uuid;

const SECRET: &[u8] = b"service-log-retention-test-signing-secret";
const ISSUER: &str = "https://artifact-rpc-test.invalid";
const CONTENT: &[u8] = b"deadline artifact fixture\n";
const AUDIENCE: &str = "/hephaestus.artifact.v1.ArtifactService/StreamArtifact";
const CREATE_AUDIENCE: &str = "/hephaestus.identity.v1.IdentityService/CreateBrowserSession";

pub(super) fn opaque(id: Uuid) -> OpaqueId {
    OpaqueId {
        value: id.to_string(),
        ..Default::default()
    }
}

pub(super) fn session_token(user_id: Uuid, sid: BrowserSessionSid) -> String {
    let now = now_seconds();
    encode(
        &Header::new(Algorithm::HS256),
        &json!({
            "iss": "hephaestus-web-mediator",
            "sub": user_id.to_string(),
            "aud": AUDIENCE,
            "iat": now,
            "nbf": now,
            "exp": now + 25,
            "jti": Uuid::new_v4().to_string(),
            "sid": sid.to_protocol_string(),
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(SECRET)),
    )
    .expect("sign artifact session assertion")
}

pub(super) fn bootstrap_token(issuer: &str, subject: &str) -> String {
    let now = now_seconds();
    encode(
        &Header::new(Algorithm::HS256),
        &json!({
            "iss": "hephaestus-web-mediator",
            "sub": "hephaestus-web-mediator",
            "aud": CREATE_AUDIENCE,
            "iat": now,
            "nbf": now,
            "exp": now + 25,
            "jti": Uuid::new_v4().to_string(),
            "actor_kind": "verified_oidc_bootstrap",
            "oidc_iss": issuer,
            "oidc_sub": subject,
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(SECRET)),
    )
    .expect("sign artifact browser-session bootstrap assertion")
}

// Keep the relational fixture together so the integration test exercises the
// same artifact records and storage path that the production adapter reads.
#[allow(clippy::too_many_lines)]
pub(super) async fn seed_artifact(pool: &PgPool, root: &Path) -> (Uuid, Uuid, String, String) {
    let user = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let build = Uuid::new_v4();
    let release = Uuid::new_v4();
    let artifact = Uuid::new_v4();
    let storage_key = Uuid::new_v4();
    let issuer = format!("{ISSUER}/{user}");
    let subject = format!("subject-{user}");
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
        .bind(user)
        .bind(format!("artifact deadline user {user}"))
        .execute(pool)
        .await
        .expect("seed artifact user");
    sqlx::query("INSERT INTO external_identities (user_id, issuer, subject, provider_metadata) VALUES ($1, $2, $3, '{}'::jsonb)")
            .bind(user)
            .bind(&issuer)
            .bind(&subject)
            .execute(pool)
            .await
            .expect("seed artifact external identity");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("artifact deadline org {user}"))
        .execute(pool)
        .await
        .expect("seed artifact organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
             VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(user)
    .execute(pool)
    .await
    .expect("seed artifact membership");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("deadline project {user}"))
        .execute(pool)
        .await
        .expect("seed artifact project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(repository)
        .bind(project)
        .bind(format!("deadline repository {user}"))
        .execute(pool)
        .await
        .expect("seed artifact repository");
    sqlx::query(
            "INSERT INTO build_requests
             (id, repository_id, source_commit, source_ref, build_definition_hash, state, created_by)
             VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
        )
        .bind(build)
        .bind(repository)
        .bind("a".repeat(40))
        .bind([1_u8; 32].as_slice())
        .bind(user)
        .execute(pool)
        .await
        .expect("seed artifact build");
    sqlx::query(
        "INSERT INTO releases
             (id, repository_id, version, source_commit, source_ref, build_request_id,
              build_definition_hash, configuration, configuration_hash, manifest_hash,
              state, publication_actor_id, published_at)
             VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5, '{}', $6, $7,
                     'published', $8, now())",
    )
    .bind(release)
    .bind(repository)
    .bind("a".repeat(40))
    .bind(build)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .bind(user)
    .execute(pool)
    .await
    .expect("seed artifact release");
    let content_hash: [u8; 32] = Sha256::digest(CONTENT).into();
    sqlx::query(
        "INSERT INTO release_artifacts
             (id, release_id, path, kind, mode, content_hash, size_bytes, media_type, storage_key)
             VALUES ($1, $2, 'deadline.txt', 'file', 292, $3, $4, 'text/plain', $5)",
    )
    .bind(artifact)
    .bind(release)
    .bind(content_hash.as_slice())
    .bind(i64::try_from(CONTENT.len()).expect("content length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed artifact metadata");
    let artifact_root = root.join("release-artifacts");
    fs::create_dir_all(&artifact_root).expect("create artifact root");
    fs::write(
        artifact_root.join(storage_key.simple().to_string()),
        CONTENT,
    )
    .expect("write artifact content");
    (artifact, user, issuer, subject)
}
