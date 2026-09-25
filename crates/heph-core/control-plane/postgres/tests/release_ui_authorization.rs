//! Real application-role inspection of immutable release UI bindings.

use authz_postgres::begin_actor_transaction;
use control_plane_postgres::release::{ReleaseApplication, ReleaseError};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::ui::UiMediaType;
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

#[tokio::test]
#[allow(clippy::too_many_lines)] // One disposable fixture covers the complete reader matrix.
#[allow(clippy::cognitive_complexity)] // Keep the role and malformed-row matrix in one test.
async fn get_release_inspects_ui_rows_through_application_role() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping release UI authorization: test URL is unset");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(12)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL");
    sqlx::migrate!("../../../../migrations")
        .run(&pool)
        .await
        .expect("apply migrations");

    let owner = UserId::new();
    let outsider = UserId::new();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    seed_identity_and_repository(&pool, owner, outsider, organization, project, repository).await;

    let valid = seed_release(&pool, owner, repository, 1).await;
    let agent_id = seed_agent(&pool, repository, valid.release_id, 1).await;
    let html_artifact = seed_artifact(&pool, valid.release_id, 1, "text/html").await;
    seed_ui_source(&pool, &valid).await;
    seed_descriptor(
        &pool,
        valid.release_id,
        "docs",
        "static",
        "docs",
        "index.html",
    )
    .await;
    sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'docs', 'index.html', $2, 'file', 'text/html')",
    )
    .bind(valid.release_id)
    .bind(html_artifact)
    .execute(&pool)
    .await
    .expect("seed static UI file");
    seed_descriptor(
        &pool,
        valid.release_id,
        "service",
        "managed_service",
        "service",
        "index.html",
    )
    .await;
    sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'service', 'gateway', '/service', $2)",
    )
    .bind(valid.release_id)
    .bind(agent_id)
    .execute(&pool)
    .await
    .expect("seed managed UI service");
    sqlx::query(
        "INSERT INTO release_ui_api_bindings
         (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
         VALUES ($1, 'service', 'health', 'gateway', 'GET', '/health', $2)",
    )
    .bind(valid.release_id)
    .bind(agent_id)
    .execute(&pool)
    .await
    .expect("seed UI API binding");
    publish(&pool, valid.release_id, owner).await;

    let legacy = seed_release(&pool, owner, repository, 2).await;
    publish(&pool, legacy.release_id, owner).await;

    let missing_managed = seed_release(&pool, owner, repository, 3).await;
    seed_ui_source(&pool, &missing_managed).await;
    seed_descriptor(
        &pool,
        missing_managed.release_id,
        "missing",
        "managed_service",
        "missing",
        "index.html",
    )
    .await;
    publish(&pool, missing_managed.release_id, owner).await;

    let non_html = seed_release(&pool, owner, repository, 4).await;
    let plain_artifact = seed_artifact(&pool, non_html.release_id, 4, "text/plain").await;
    seed_ui_source(&pool, &non_html).await;
    seed_descriptor(
        &pool,
        non_html.release_id,
        "plain",
        "static",
        "plain",
        "index.html",
    )
    .await;
    sqlx::query(
        "INSERT INTO release_ui_static_files
         (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
         VALUES ($1, 'plain', 'index.html', $2, 'file', 'text/plain')",
    )
    .bind(non_html.release_id)
    .bind(plain_artifact)
    .execute(&pool)
    .await
    .expect("seed non-HTML static file");
    publish(&pool, non_html.release_id, owner).await;

    let overflow = seed_release(&pool, owner, repository, 5).await;
    let overflow_agent = seed_agent(&pool, repository, overflow.release_id, 5).await;
    seed_ui_source(&pool, &overflow).await;
    for index in 0..17 {
        let key = format!("overflow-{index}");
        seed_descriptor(
            &pool,
            overflow.release_id,
            &key,
            "managed_service",
            &format!("route-{index}"),
            "index.html",
        )
        .await;
        sqlx::query(
            "INSERT INTO release_ui_managed_services
             (release_id, ui_key, gateway_name, route, release_agent_id)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(overflow.release_id)
        .bind(&key)
        .bind(format!("gateway-{index}"))
        .bind(format!("/service-{index}"))
        .bind(overflow_agent)
        .execute(&pool)
        .await
        .expect("seed overflow managed UI service");
    }
    publish(&pool, overflow.release_id, owner).await;

    let file_overflow = seed_release(&pool, owner, repository, 6).await;
    let overflow_artifact = seed_artifact(&pool, file_overflow.release_id, 6, "text/html").await;
    seed_ui_source(&pool, &file_overflow).await;
    seed_descriptor(
        &pool,
        file_overflow.release_id,
        "files",
        "static",
        "files",
        "index.html",
    )
    .await;
    for index in 0..257 {
        let route = if index == 0 {
            String::from("index.html")
        } else {
            format!("file-{index}.js")
        };
        sqlx::query(
            "INSERT INTO release_ui_static_files
             (release_id, ui_key, route, artifact_id, artifact_kind, artifact_media_type)
             VALUES ($1, 'files', $2, $3, 'file', 'text/html')",
        )
        .bind(file_overflow.release_id)
        .bind(route)
        .bind(overflow_artifact)
        .execute(&pool)
        .await
        .expect("seed per-UI static file overflow");
    }
    publish(&pool, file_overflow.release_id, owner).await;

    let api_overflow = seed_release(&pool, owner, repository, 7).await;
    let api_agent = seed_agent(&pool, repository, api_overflow.release_id, 7).await;
    seed_ui_source(&pool, &api_overflow).await;
    seed_descriptor(
        &pool,
        api_overflow.release_id,
        "apis",
        "managed_service",
        "apis",
        "index.html",
    )
    .await;
    sqlx::query(
        "INSERT INTO release_ui_managed_services
         (release_id, ui_key, gateway_name, route, release_agent_id)
         VALUES ($1, 'apis', 'gateway', '/service', $2)",
    )
    .bind(api_overflow.release_id)
    .bind(api_agent)
    .execute(&pool)
    .await
    .expect("seed API overflow managed service");
    for index in 0..17 {
        sqlx::query(
            "INSERT INTO release_ui_api_bindings
             (release_id, ui_key, api_key, gateway_name, method, route, release_agent_id)
             VALUES ($1, 'apis', $2, 'gateway', 'GET', $3, $4)",
        )
        .bind(api_overflow.release_id)
        .bind(format!("api-{index}"))
        .bind(format!("/api-{index}"))
        .bind(api_agent)
        .execute(&pool)
        .await
        .expect("seed per-UI API overflow");
    }
    publish(&pool, api_overflow.release_id, owner).await;

    let app_pool = PgPoolOptions::new()
        .max_connections(8)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(connection)
                    .await
                    .map(|_| ())
            })
        })
        .connect(&database_url)
        .await
        .expect("connect application-role PostgreSQL pool");
    let (current_user, is_superuser, bypasses_rls): (String, bool, bool) = sqlx::query_as(
        "SELECT current_user, rolsuper, rolbypassrls
         FROM pg_roles WHERE rolname = current_user",
    )
    .fetch_one(&app_pool)
    .await
    .expect("inspect application database role");
    assert_eq!(current_user, "hephaestus_app");
    assert!(!is_superuser);
    assert!(!bypasses_rls);

    let application = ReleaseApplication::new(app_pool.clone());
    let owner_identity = identity(owner);
    let outsider_identity = identity(outsider);
    let detail = application
        .get_release(&owner_identity, valid.release_id)
        .await
        .expect("owner release UI detail");
    assert_eq!(detail.ui_descriptors.len(), 2);
    assert_eq!(detail.ui_descriptors[0].key.as_str(), "docs");
    assert_eq!(detail.ui_descriptors[1].key.as_str(), "service");
    let control_plane_postgres::release::ui::ReleaseUiContent::Static {
        files: static_files,
    } = &detail.ui_descriptors[0].content
    else {
        panic!("expected static UI");
    };
    assert_eq!(static_files[0].artifact_id, html_artifact);
    assert_eq!(static_files[0].media_type, UiMediaType::TextHtml);
    let control_plane_postgres::release::ui::ReleaseUiContent::ManagedService {
        release_agent_id: managed,
        ..
    } = &detail.ui_descriptors[1].content
    else {
        panic!("expected managed UI");
    };
    assert_eq!(*managed, agent_id);
    assert_eq!(detail.ui_descriptors[1].apis[0].key.as_str(), "health");
    assert_eq!(detail.ui_descriptors[1].apis[0].release_agent_id, agent_id);

    let legacy_detail = application
        .get_release(&owner_identity, legacy.release_id)
        .await
        .expect("legacy release detail");
    assert!(legacy_detail.ui_descriptors.is_empty());
    assert!(matches!(
        application
            .get_release(&outsider_identity, valid.release_id)
            .await,
        Err(ReleaseError::NotFound)
    ));
    assert!(matches!(
        application
            .get_release(&owner_identity, missing_managed.release_id)
            .await,
        Err(ReleaseError::InvalidStoredData)
    ));
    assert!(matches!(
        application
            .get_release(&owner_identity, non_html.release_id)
            .await,
        Err(ReleaseError::InvalidStoredData)
    ));
    assert!(matches!(
        application
            .get_release(&owner_identity, overflow.release_id)
            .await,
        Err(ReleaseError::InvalidStoredData)
    ));
    assert!(matches!(
        application
            .get_release(&owner_identity, file_overflow.release_id)
            .await,
        Err(ReleaseError::InvalidStoredData)
    ));
    assert!(matches!(
        application
            .get_release(&owner_identity, api_overflow.release_id)
            .await,
        Err(ReleaseError::InvalidStoredData)
    ));

    let mut transaction = begin_actor_transaction(&app_pool, &owner_identity)
        .await
        .expect("owner actor transaction");
    let visible_owner: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1")
            .bind(valid.release_id)
            .fetch_one(&mut *transaction)
            .await
            .expect("owner application-role UI descriptor visibility");
    assert_eq!(visible_owner, 2);
    transaction
        .rollback()
        .await
        .expect("rollback actor transaction");

    let mut outsider_transaction = begin_actor_transaction(&app_pool, &outsider_identity)
        .await
        .expect("outsider actor transaction");
    let visible_outsider: i64 =
        sqlx::query_scalar("SELECT count(*) FROM release_ui_descriptors WHERE release_id = $1")
            .bind(valid.release_id)
            .fetch_one(&mut *outsider_transaction)
            .await
            .expect("outsider application-role UI descriptor visibility");
    assert_eq!(visible_outsider, 0);
    outsider_transaction
        .rollback()
        .await
        .expect("rollback outsider actor transaction");
    app_pool.close().await;
    pool.close().await;
}

// These names mirror the database columns to keep fixture bindings auditable.
#[allow(clippy::struct_field_names)]
struct ReleaseFixture {
    build_id: Uuid,
    release_id: Uuid,
    receive_id: Uuid,
    revision_id: Uuid,
}

async fn seed_identity_and_repository(
    pool: &PgPool,
    owner: UserId,
    outsider: UserId,
    organization: Uuid,
    project: Uuid,
    repository: Uuid,
) {
    for (user, name) in [(owner, "ui-owner"), (outsider, "ui-outsider")] {
        sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, $2)")
            .bind(user.as_uuid())
            .bind(name)
            .execute(pool)
            .await
            .expect("seed UI user");
    }
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'ui-org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed UI organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed UI owner membership");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'ui-project')")
        .bind(project)
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed UI project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'ui-repository')")
        .bind(repository)
        .bind(project)
        .execute(pool)
        .await
        .expect("seed UI repository");
}

async fn seed_release(pool: &PgPool, owner: UserId, repository: Uuid, seed: u8) -> ReleaseFixture {
    let build_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let receive_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let source_commit = format!("{seed:040x}");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build_id)
    .bind(repository)
    .bind(&source_commit)
    .bind([seed; 32].as_slice())
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed UI build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref, build_request_id,
          build_definition_hash, configuration, configuration_hash, manifest_hash, state)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}', $7, $8, 'draft')",
    )
    .bind(release_id)
    .bind(repository)
    .bind(format!("v1.0.{seed}"))
    .bind(&source_commit)
    .bind(build_id)
    .bind([seed; 32].as_slice())
    .bind([seed + 10; 32].as_slice())
    .bind([seed + 20; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed UI release");
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'release-ui-test', 'accepted', now())",
    )
    .bind(receive_id)
    .bind(repository)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed UI receive");
    ReleaseFixture {
        build_id,
        release_id,
        receive_id,
        revision_id,
    }
}

async fn seed_ui_source(pool: &PgPool, fixture: &ReleaseFixture) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, entry_kind, manifest_oid,
          actual_size_bytes, source_sha256, status, normalized_ui_config,
          normalized_ui_hash, diagnostics)
         SELECT $1, repository_id, $2, source_commit, 'blob', $3,
                1, $4, 'valid', $5, $6, '[]'
         FROM build_requests WHERE id = $7",
    )
    .bind(fixture.revision_id)
    .bind(fixture.receive_id)
    .bind(format!("{:040x}", fixture.revision_id.as_u128()))
    .bind([3_u8; 32].as_slice())
    .bind(json!({"version": 1, "uis": []}))
    .bind([4_u8; 32].as_slice())
    .bind(fixture.build_id)
    .execute(pool)
    .await
    .expect("seed UI source revision");
    sqlx::query(
        "INSERT INTO build_request_ui_source_manifests
         (build_request_id, repository_id, source_commit, source_manifest_revision_id)
         SELECT $1, repository_id, source_commit, $2
         FROM build_requests WHERE id = $1",
    )
    .bind(fixture.build_id)
    .bind(fixture.revision_id)
    .execute(pool)
    .await
    .expect("seed UI source link");
    sqlx::query(
        "INSERT INTO release_ui_source_snapshots
         (release_id, build_request_id, source_manifest_revision_id)
         VALUES ($1, $2, $3)",
    )
    .bind(fixture.release_id)
    .bind(fixture.build_id)
    .bind(fixture.revision_id)
    .execute(pool)
    .await
    .expect("seed UI release source snapshot");
}

async fn seed_descriptor(
    pool: &PgPool,
    release_id: Uuid,
    key: &str,
    content_kind: &str,
    route_base: &str,
    entrypoint: &str,
) {
    sqlx::query(
        "INSERT INTO release_ui_descriptors
         (release_id, ui_key, scope, label, icon, presentation, route_base,
          entrypoint, ui_kit_version, cache, content_kind)
         VALUES ($1, $2, 'repository', $3, 'app', 'full_page', $4, $5, 1,
                 'no_store', $6)",
    )
    .bind(release_id)
    .bind(key)
    .bind(key)
    .bind(route_base)
    .bind(entrypoint)
    .bind(content_kind)
    .execute(pool)
    .await
    .expect("seed UI descriptor");
}

async fn seed_artifact(pool: &PgPool, release_id: Uuid, seed: u8, media_type: &str) -> Uuid {
    let artifact_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO release_artifacts
         (id, release_id, path, kind, mode, content_hash, size_bytes, media_type, storage_key)
         VALUES ($1, $2, $3, 'file', 420, $4, 1, $5, $6)",
    )
    .bind(artifact_id)
    .bind(release_id)
    .bind(format!("ui-{seed}.html"))
    .bind([seed; 32].as_slice())
    .bind(media_type)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("seed UI artifact");
    artifact_id
}

async fn seed_agent(pool: &PgPool, repository: Uuid, release_id: Uuid, seed: u8) -> Uuid {
    let family_id = Uuid::new_v4();
    let agent_id = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_families (id, repository_id, agent_key) VALUES ($1, $2, $3)")
        .bind(family_id)
        .bind(repository)
        .bind(format!("ui-agent-{seed}"))
        .execute(pool)
        .await
        .expect("seed UI agent family");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name, runtime_contract,
          runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, $4, 'UI Agent', $5, $6, '[]', '[]', false)",
    )
    .bind(agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(format!("ui-agent-{seed}"))
    .bind(json!({
        "policy_ceiling": {"vcpus": 1, "memory_mib": 128, "network": "disabled"}
    }))
    .bind([seed; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed UI release agent");
    agent_id
}

async fn publish(pool: &PgPool, release_id: Uuid, owner: UserId) {
    sqlx::query(
        "UPDATE releases
         SET state = 'published', publication_actor_id = $2, published_at = now()
         WHERE id = $1",
    )
    .bind(release_id)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("publish UI test release");
}

fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://release-ui-test.example",
        format!("subject-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}
