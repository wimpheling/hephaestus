//! Real application-role coverage for UI-aware manual build identity.

use control_plane_postgres::build::{BuildApplication, BuildError, RequestBuild};
use control_plane_postgres::connect_app;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::fmt::Write as _;
use uuid::Uuid;

const CONFIG: &str = r#"
version = 2
[agent]
name = "Manual UI test agent"
key = "manual-ui"
[build]
image = { key = "manual-builder" }
command = "/bin/build"
working_directory = "/source"
triggers = ["refs/heads/main"]
[build.resources]
vcpus = 1
memory_mib = 256
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "bin/app"
kind = "executable"
[guest]
image = { key = "manual-runtime" }
command = "bin/app"
arguments = []
working_directory = "bin"
[resources]
vcpus = 1
memory_mib = 128
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = false
[network]
profile = "disabled"
[triggers]
push = false
"#;

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn manual_build_ui_basic_matrix_uses_application_role() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping manual UI build matrix: test URL is unset");
        return;
    };
    let bootstrap = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL bootstrap pool");
    sqlx::migrate!("../../migrations")
        .run(&bootstrap)
        .await
        .expect("apply migrations");

    let owner = UserId::new();
    let repository = Uuid::new_v4();
    let project = Uuid::new_v4();
    let organization = Uuid::new_v4();
    seed_identity(&bootstrap, owner, organization, project, repository).await;
    seed_images(&bootstrap).await;

    let parsed = agent_config::parse(CONFIG.as_bytes());
    let config = parsed.config.expect("valid test configuration");
    let normalized_config_hash = parsed
        .normalized_hash
        .expect("normalized configuration hash");
    let configuration = serde_json::to_value(&config).expect("serialize configuration");
    let build = config.build.as_ref().expect("build declaration");
    let base_hash =
        agent_config::build_identity::base_build_definition_hash(build).expect("base build hash");
    let ui_hash = [4_u8; 32];
    let derived_hash =
        agent_config::build_identity::ui_build_definition_hash(base_hash, ui_hash, None);
    let parallel_identity = identity(owner);
    let duplicate_identity = identity(owner);
    let identity = identity(owner);
    let app_pool = connect_app(&database_url, 4).await.expect("app pool");
    let application = BuildApplication::new(app_pool);

    let valid_commit = "a".repeat(40);
    let valid_receive = Uuid::new_v4();
    seed_source(
        &bootstrap,
        repository,
        valid_receive,
        &valid_commit,
        &configuration,
        &parsed.hash,
        &normalized_config_hash,
    )
    .await;
    seed_valid_ui(
        &bootstrap,
        repository,
        valid_receive,
        &valid_commit,
        ui_hash,
    )
    .await;

    let first = application
        .request_build(
            &identity,
            request(
                repository,
                &valid_commit,
                base_hash,
                &normalized_config_hash,
            ),
        )
        .await
        .expect("base hash accepted for valid UI");
    let second = application
        .request_build(
            &duplicate_identity,
            request(
                repository,
                &valid_commit,
                derived_hash,
                &normalized_config_hash,
            ),
        )
        .await
        .expect("derived hash accepted for valid UI");
    assert_eq!(first.id, second.id, "base and derived callers deduplicate");
    let duplicate_receipt_events: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM application_events
         WHERE occurrence_id = $1 AND aggregate_type = 'build'
           AND aggregate_id = $2
           AND scope_kind = 'repository' AND scope_id = $3 AND actor_id = $4",
    )
    .bind(duplicate_identity.idempotency_id.as_uuid())
    .bind(first.id)
    .bind(repository)
    .bind(owner.as_uuid())
    .fetch_one(&bootstrap)
    .await
    .expect("deduplicated build receipt event");
    assert_eq!(duplicate_receipt_events, 1);
    let retried = application
        .request_build(
            &duplicate_identity,
            request(
                repository,
                &valid_commit,
                derived_hash,
                &normalized_config_hash,
            ),
        )
        .await
        .expect("repeated deduplicated build request");
    assert_eq!(retried.id, first.id);
    let repeated_receipt_events: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM application_events
         WHERE occurrence_id = $1 AND aggregate_type = 'build'
           AND aggregate_id = $2
           AND scope_kind = 'repository' AND scope_id = $3 AND actor_id = $4",
    )
    .bind(duplicate_identity.idempotency_id.as_uuid())
    .bind(first.id)
    .bind(repository)
    .bind(owner.as_uuid())
    .fetch_one(&bootstrap)
    .await
    .expect("repeated deduplicated build receipt count");
    assert_eq!(repeated_receipt_events, 1);
    assert_eq!(outbox_count(&bootstrap, &valid_commit).await, 1);
    let (parallel_first, parallel_second) = tokio::join!(
        application.request_build(
            &parallel_identity,
            request(
                repository,
                &valid_commit,
                derived_hash,
                &normalized_config_hash,
            ),
        ),
        application.request_build(
            &parallel_identity,
            request(
                repository,
                &valid_commit,
                derived_hash,
                &normalized_config_hash,
            ),
        ),
    );
    let parallel_first = parallel_first.expect("first concurrent deduplicated build request");
    let parallel_second = parallel_second.expect("second concurrent deduplicated build request");
    assert_eq!(parallel_first.id, first.id);
    assert_eq!(parallel_second.id, first.id);
    let parallel_receipt_events: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM application_events
         WHERE occurrence_id = $1 AND aggregate_type = 'build'
           AND aggregate_id = $2
           AND scope_kind = 'repository' AND scope_id = $3 AND actor_id = $4",
    )
    .bind(parallel_identity.idempotency_id.as_uuid())
    .bind(first.id)
    .bind(repository)
    .bind(owner.as_uuid())
    .fetch_one(&bootstrap)
    .await
    .expect("concurrent deduplicated build receipt count");
    assert_eq!(parallel_receipt_events, 1);
    assert_eq!(outbox_count(&bootstrap, &valid_commit).await, 1);
    let stored_hash: Vec<u8> =
        sqlx::query_scalar("SELECT build_definition_hash FROM build_requests WHERE id = $1")
            .bind(first.id)
            .fetch_one(&bootstrap)
            .await
            .expect("stored derived build hash");
    assert_eq!(stored_hash, derived_hash);
    let link: (Uuid, Uuid, String, String) = sqlx::query_as(
        "SELECT build_request_id, repository_id, source_commit, source_status
         FROM build_request_ui_source_manifests WHERE build_request_id = $1",
    )
    .bind(first.id)
    .fetch_one(&bootstrap)
    .await
    .expect("exact UI build link");
    assert_eq!(
        link,
        (
            first.id,
            repository,
            valid_commit.clone(),
            "valid".to_owned()
        )
    );
    let valid_event: serde_json::Value = sqlx::query_scalar(
        "SELECT payload
         FROM outbox
         WHERE payload->>'source_commit' = $1
         ORDER BY occurred_at, id
         LIMIT 1",
    )
    .bind(&valid_commit)
    .fetch_one(&bootstrap)
    .await
    .expect("valid build event");
    assert_eq!(outbox_count(&bootstrap, &valid_commit).await, 1);
    let first_id = first.id.to_string();
    let expected_hash = hex_hash(derived_hash);
    assert_eq!(
        valid_event["build_request_id"].as_str(),
        Some(first_id.as_str())
    );
    assert_eq!(
        valid_event["build_definition_hash"].as_str(),
        Some(expected_hash.as_str())
    );

    let invalid_commit = "b".repeat(40);
    let invalid_receive = Uuid::new_v4();
    seed_source(
        &bootstrap,
        repository,
        invalid_receive,
        &invalid_commit,
        &configuration,
        &parsed.hash,
        &normalized_config_hash,
    )
    .await;
    seed_invalid_ui(&bootstrap, repository, invalid_receive, &invalid_commit).await;
    let invalid = application
        .request_build(
            &identity,
            request(
                repository,
                &invalid_commit,
                base_hash,
                &normalized_config_hash,
            ),
        )
        .await;
    assert!(matches!(invalid, Err(BuildError::FailedPrecondition)));
    assert_eq!(
        build_count(&bootstrap, repository, &invalid_commit).await,
        0
    );
    assert_eq!(outbox_count(&bootstrap, &invalid_commit).await, 0);

    let absent_commit = "c".repeat(40);
    let absent_receive = Uuid::new_v4();
    seed_source(
        &bootstrap,
        repository,
        absent_receive,
        &absent_commit,
        &configuration,
        &parsed.hash,
        &normalized_config_hash,
    )
    .await;
    let absent = application
        .request_build(
            &identity,
            request(
                repository,
                &absent_commit,
                base_hash,
                &normalized_config_hash,
            ),
        )
        .await
        .expect("absent UI preserves legacy identity");
    let absent_hash: Vec<u8> =
        sqlx::query_scalar("SELECT build_definition_hash FROM build_requests WHERE id = $1")
            .bind(absent.id)
            .fetch_one(&bootstrap)
            .await
            .expect("legacy build hash");
    assert_eq!(absent_hash, base_hash);
    assert_eq!(outbox_count(&bootstrap, &absent_commit).await, 1);

    seed_valid_ui(
        &bootstrap,
        repository,
        absent_receive,
        &absent_commit,
        ui_hash,
    )
    .await;
    let absent_derived = application
        .request_build(
            &identity,
            request(
                repository,
                &absent_commit,
                base_hash,
                &normalized_config_hash,
            ),
        )
        .await
        .expect("legacy build accepts newly captured UI as a new identity");
    assert_ne!(absent_derived.id, absent.id);
    let absent_derived_hash: Vec<u8> = sqlx::query_scalar(
        "SELECT build_definition_hash
         FROM build_requests
         WHERE id = $1",
    )
    .bind(absent_derived.id)
    .fetch_one(&bootstrap)
    .await
    .expect("derived legacy build hash");
    assert_eq!(absent_derived_hash, derived_hash);
    let old_identity: (Uuid, Vec<u8>) = sqlx::query_as(
        "SELECT id, build_definition_hash
         FROM build_requests
         WHERE id = $1",
    )
    .bind(absent.id)
    .fetch_one(&bootstrap)
    .await
    .expect("legacy build identity remains stable");
    assert_eq!(old_identity, (absent.id, base_hash.to_vec()));
    let old_link_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM build_request_ui_source_manifests
         WHERE build_request_id = $1",
    )
    .bind(absent.id)
    .fetch_one(&bootstrap)
    .await
    .expect("legacy build remains unlinked");
    assert_eq!(old_link_count, 0);
    let new_link_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM build_request_ui_source_manifests
         WHERE build_request_id = $1 AND source_status = 'valid'",
    )
    .bind(absent_derived.id)
    .fetch_one(&bootstrap)
    .await
    .expect("derived legacy build UI link");
    assert_eq!(new_link_count, 1);
    let absent_deduplicated = application
        .request_build(
            &identity,
            request(
                repository,
                &absent_commit,
                derived_hash,
                &normalized_config_hash,
            ),
        )
        .await
        .expect("derived legacy build deduplicates");
    assert_eq!(absent_deduplicated.id, absent_derived.id);
    assert_eq!(outbox_count(&bootstrap, &absent_commit).await, 2);
    assert_eq!(outbox_count_for_build(&bootstrap, absent.id).await, 1);
    assert_eq!(
        outbox_count_for_build(&bootstrap, absent_derived.id).await,
        1
    );

    let wrong = application
        .request_build(
            &identity,
            request(repository, &valid_commit, [9; 32], &normalized_config_hash),
        )
        .await;
    assert!(matches!(wrong, Err(BuildError::FailedPrecondition)));
    assert_eq!(build_count(&bootstrap, repository, &valid_commit).await, 1);
    assert_eq!(outbox_count(&bootstrap, &valid_commit).await, 1);
    assert_eq!(outbox_count_for_build(&bootstrap, first.id).await, 1);
}

fn request(
    repository_id: Uuid,
    source_commit: &str,
    build_definition_hash: [u8; 32],
    configuration_hash: &agent_config::ConfigHash,
) -> RequestBuild {
    RequestBuild {
        repository_id,
        source_commit: source_commit.to_owned(),
        build_definition_hash,
        configuration_hash: decode_hash(configuration_hash.as_str()),
    }
}

fn decode_hash(value: &str) -> [u8; 32] {
    let mut hash = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        hash[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
    }
    hash
}

const fn nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => 0,
    }
}

fn hex_hash(value: [u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in value {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn identity(user_id: UserId) -> AuthenticatedIdentity {
    AuthenticatedIdentity::new(
        user_id,
        "https://manual-ui-build.example",
        format!("manual-ui-{user_id}"),
        json!({"email_verified": true}),
        RequestId::new(),
    )
}

async fn seed_identity(
    pool: &PgPool,
    owner: UserId,
    organization: Uuid,
    project: Uuid,
    repository: Uuid,
) {
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'manual-ui-owner')")
        .bind(owner.as_uuid())
        .execute(pool)
        .await
        .expect("user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'manual-ui-org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("organization membership");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'manual-ui-project')",
    )
    .bind(project)
    .bind(organization)
    .execute(pool)
    .await
    .expect("project");
    sqlx::query(
        "INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'manual-ui-repository')",
    )
    .bind(repository)
    .bind(project)
    .execute(pool)
    .await
    .expect("repository");
    sqlx::query(
        "INSERT INTO repository_managers (repository_id, user_id)
         VALUES ($1, $2)",
    )
    .bind(repository)
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("repository manager");
}

async fn seed_images(pool: &PgPool) {
    for (key, byte) in [("manual-builder", b'a'), ("manual-runtime", b'b')] {
        let reference = format!(
            "manual-{key}@sha256:{}",
            char::from(byte).to_string().repeat(64)
        );
        sqlx::query(
            "INSERT INTO oci_images
             (id, key, display_name, image_reference, toolchains, architectures,
              availability_state, provenance, platform_policy_version, role)
             VALUES ($1, $2, $3, $4, '[]', ARRAY['x86_64'], 'available', '{}', 'test/v1', 'execution')",
        )
        .bind(Uuid::new_v4())
        .bind(key)
        .bind(key)
        .bind(reference)
        .execute(pool)
        .await
        .expect("OCI image");
    }
}

async fn seed_source(
    pool: &PgPool,
    repository: Uuid,
    receive: Uuid,
    commit: &str,
    config: &serde_json::Value,
    source_hash: &agent_config::ConfigHash,
    normalized_hash: &agent_config::ConfigHash,
) {
    sqlx::query(
        "INSERT INTO git_receives (id, repository_id, principal, status)
         VALUES ($1, $2, 'manual-ui-test', 'accepted')",
    )
    .bind(receive)
    .bind(repository)
    .execute(pool)
    .await
    .expect("receive");
    sqlx::query(
        "INSERT INTO git_refs (repository_id, git_ref, commit_sha, updated_by_receive_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(repository)
    .bind(format!("refs/heads/manual-{}", &commit[..8]))
    .bind(commit)
    .bind(receive)
    .execute(pool)
    .await
    .expect("Git ref");
    sqlx::query(
        "INSERT INTO agent_config_revisions
         (id, repository_id, receive_id, commit_sha, config_hash, normalized_config_hash,
          schema_version, status, config)
         VALUES ($1, $2, $3, $4, $5, $6, 2, 'valid', $7)",
    )
    .bind(Uuid::new_v4())
    .bind(repository)
    .bind(receive)
    .bind(commit)
    .bind(source_hash.as_str())
    .bind(normalized_hash.as_str())
    .bind(config)
    .execute(pool)
    .await
    .expect("agent configuration revision");
}

async fn seed_valid_ui(
    pool: &PgPool,
    repository: Uuid,
    receive: Uuid,
    commit: &str,
    hash: [u8; 32],
) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, manifest_path, entry_kind,
          manifest_oid, actual_size_bytes, source_sha256, status, requires_gateways,
          normalized_ui_config, normalized_ui_hash, diagnostics)
         VALUES ($1, $2, $3, $4, 'heph.ui.toml', 'blob', repeat('a', 40), 1,
                 $5, 'valid', false, '{}', $6, '[]')",
    )
    .bind(Uuid::new_v4())
    .bind(repository)
    .bind(receive)
    .bind(commit)
    .bind([3_u8; 32].as_slice())
    .bind(hash.as_slice())
    .execute(pool)
    .await
    .expect("valid UI capture");
}

async fn seed_invalid_ui(pool: &PgPool, repository: Uuid, receive: Uuid, commit: &str) {
    sqlx::query(
        "INSERT INTO ui_source_manifest_revisions
         (id, repository_id, receive_id, source_commit, manifest_path, entry_kind,
          manifest_oid, status, requires_gateways, diagnostics)
         VALUES ($1, $2, $3, $4, 'heph.ui.toml', 'blob', repeat('b', 40),
                 'invalid', false, '[{\"code\":\"invalid\"}]')",
    )
    .bind(Uuid::new_v4())
    .bind(repository)
    .bind(receive)
    .bind(commit)
    .execute(pool)
    .await
    .expect("invalid UI capture");
}

async fn build_count(pool: &PgPool, repository: Uuid, commit: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM build_requests WHERE repository_id = $1 AND source_commit = $2",
    )
    .bind(repository)
    .bind(commit)
    .fetch_one(pool)
    .await
    .expect("build count")
}

async fn outbox_count(pool: &PgPool, commit: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM outbox WHERE payload->>'source_commit' = $1")
        .bind(commit)
        .fetch_one(pool)
        .await
        .expect("outbox count")
}

async fn outbox_count_for_build(pool: &PgPool, build_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
         FROM outbox
         WHERE payload->>'build_request_id' = $1",
    )
    .bind(build_id.to_string())
    .fetch_one(pool)
    .await
    .expect("outbox count for build")
}
