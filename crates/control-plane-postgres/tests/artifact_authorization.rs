//! Focused artifact and release read authorization against real `PostgreSQL`.

use authz_domain::{ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, begin_actor_transaction};
use control_plane_postgres::artifact::{ArtifactApplication, ArtifactError, StreamArtifact};
use control_plane_postgres::release::{ReleaseApplication, ReleasePage};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_artifact_store::LocalArtifactStore;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{fs, os::unix::fs::PermissionsExt};
use tempfile::tempdir;
use uuid::Uuid;

const CONTENT: &[u8] = b"private artifact fixture\n";

#[tokio::test]
#[allow(clippy::too_many_lines)] // Keep this regression's cross-entrypoint assertions together.
async fn superuser_pool_still_enforces_artifact_read_authorization() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping artifact authorization regression: test URL is unset");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply migrations");

    let owner = UserId::new();
    let outsider = UserId::new();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let build = Uuid::new_v4();
    let release = Uuid::new_v4();
    let artifact = Uuid::new_v4();
    let storage_key = Uuid::new_v4();
    seed_fixture(
        &pool,
        owner,
        outsider,
        organization,
        project,
        repository,
        build,
        release,
        artifact,
        storage_key,
    )
    .await;
    let artifact_root = tempdir().expect("artifact store root");
    fs::set_permissions(artifact_root.path(), fs::Permissions::from_mode(0o700))
        .expect("private artifact root");
    fs::write(
        artifact_root.path().join(storage_key.simple().to_string()),
        CONTENT,
    )
    .expect("canonical artifact");
    fs::set_permissions(
        artifact_root.path().join(storage_key.simple().to_string()),
        fs::Permissions::from_mode(0o400),
    )
    .expect("immutable artifact");
    let store = LocalArtifactStore::new(artifact_root.path().to_owned()).expect("artifact store");
    let application = ArtifactApplication::new(pool.clone(), store, [9_u8; 32]);
    let owner_identity = identity(owner);
    let outsider_identity = identity(outsider);

    let preview = application
        .get_artifact_preview(&owner_identity, artifact, 1024)
        .await
        .expect("owner preview authorization");
    assert_eq!(preview.utf8_contents.as_bytes(), CONTENT);
    let mut stream = application
        .stream_artifact(
            &owner_identity,
            StreamArtifact {
                artifact_id: artifact,
                resume_cursor: None,
                max_total_bytes: 1024,
                max_chunk_bytes: 1024,
            },
        )
        .await
        .expect("owner stream authorization");
    let chunk = stream
        .receiver
        .recv()
        .await
        .expect("owner stream chunk")
        .expect("owner stream result");
    assert_eq!(chunk.contents, CONTENT);

    assert!(matches!(
        application
            .get_artifact_preview(&outsider_identity, artifact, 1024)
            .await,
        Err(ArtifactError::NotFound)
    ));
    assert!(matches!(
        application
            .stream_artifact(
                &outsider_identity,
                StreamArtifact {
                    artifact_id: artifact,
                    resume_cursor: None,
                    max_total_bytes: 1024,
                    max_chunk_bytes: 1024,
                },
            )
            .await,
        Err(ArtifactError::NotFound)
    ));

    let releases = ReleaseApplication::new(pool.clone());
    let owner_page = releases
        .list_repository_releases(
            &owner_identity,
            repository,
            ReleasePage {
                size: 10,
                after: None,
            },
        )
        .await
        .expect("owner release metadata list");
    assert_eq!(owner_page.releases.len(), 1);
    assert_eq!(owner_page.releases[0].artifact_count, 1);
    let outsider_page = releases
        .list_repository_releases(
            &outsider_identity,
            repository,
            ReleasePage {
                size: 10,
                after: None,
            },
        )
        .await
        .expect("outsider release list is nondisclosing");
    assert!(outsider_page.releases.is_empty());
    assert!(matches!(
        releases.get_release(&outsider_identity, release).await,
        Err(control_plane_postgres::release::ReleaseError::NotFound)
    ));
    let owner_detail = releases
        .get_release(&owner_identity, release)
        .await
        .expect("owner release metadata");
    assert_eq!(owner_detail.artifacts.len(), 1);
    assert_eq!(owner_detail.artifacts[0].id, artifact);

    let mut transaction = begin_actor_transaction(&pool, &outsider_identity)
        .await
        .expect("outsider authorization transaction");
    let decision = PostgresMelangeAuthorizer
        .check(
            &mut transaction,
            Subject::User(outsider),
            Permission::CanRead,
            ObjectRef::new(ObjectType::Release, release),
        )
        .await
        .expect("fail-closed evaluator decision");
    assert!(!decision.is_allowed());
    transaction
        .rollback()
        .await
        .expect("rollback authorization check");
}

fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://artifact-regression.example",
        format!("subject-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}

#[allow(clippy::too_many_arguments)]
async fn seed_fixture(
    pool: &PgPool,
    owner: UserId,
    outsider: UserId,
    organization: Uuid,
    project: Uuid,
    repository: Uuid,
    build: Uuid,
    release: Uuid,
    artifact: Uuid,
    storage_key: Uuid,
) {
    for (id, name) in [(owner, "artifact-owner"), (outsider, "artifact-outsider")] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(id.as_uuid())
            .bind(name)
            .execute(pool)
            .await
            .expect("seed user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'artifact-org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed owner membership");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'artifact-project')",
    )
    .bind(project)
    .bind(organization)
    .execute(pool)
    .await
    .expect("seed project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'artifact-private')",
    )
    .bind(repository)
    .bind(project)
    .execute(pool)
    .await
    .expect("seed repository");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build)
    .bind(repository)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed build");
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
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed release");
    let content_hash: [u8; 32] = Sha256::digest(CONTENT).into();
    sqlx::query(
        "INSERT INTO release_artifacts
         (id, release_id, path, kind, mode, content_hash, size_bytes, media_type, storage_key)
         VALUES ($1, $2, 'private.txt', 'file', 292, $3, $4, 'text/plain', $5)",
    )
    .bind(artifact)
    .bind(release)
    .bind(content_hash.as_slice())
    .bind(i64::try_from(CONTENT.len()).expect("content length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed artifact");
}
