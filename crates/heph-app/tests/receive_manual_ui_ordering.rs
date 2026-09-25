//! Receive/manual UI capture ordering through two real application-role pools.

#![cfg(feature = "test-fixtures")]

use authz_postgres::PostgresMelangeAuthorizer;
use control_plane_postgres::build::{BuildApplication, BuildError, RequestBuild};
use forge_domain::{CommitSha, GitRef, OrganizationId, ReceiveId, RefUpdate, Repository};
use forge_postgres::PgForgeRepository;
use forge_service::{CreateRepository, GitStorage};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use serde_json::json;
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{path::Path, sync::Arc, time::Duration};
use tempfile::TempDir;
use tokio::{process::Command, time::timeout};
use uuid::Uuid;

const CONFIG_TEMPLATE: &str = r#"
version = 2
[agent]
name = "Receive ordering test agent"
key = "receive-ordering"
[build]
image = { key = "__BUILD_IMAGE__" }
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
image = { key = "__RUNTIME_IMAGE__" }
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

const VALID_UI: &str = r#"
version = 1
[[uis]]
key = "docs"
scope = "global"
label = "Docs"
icon = "book"
presentation = "iframe"
route_base = "docs"
ui_kit_version = 1
cache = "no_store"
[uis.content]
kind = "static"
entrypoint = "index.html"
[[uis.content.files]]
route = "index.html"
artifact = "dist/index.html"
media_type = "text/html"
"#;

const INVALID_UI: &str = "version = 1\nunknown_field = \"invalid\"\n";

#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn receive_commits_before_manual_request_observes_ui_capture() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        eprintln!("skipping receive/manual UI ordering: test URL is unset");
        return;
    };

    run_ordering_case(&database_url, VALID_UI, true).await;
    run_ordering_case(&database_url, INVALID_UI, false).await;
}

// Keep both lock-order cases together so their barrier and authorization paths
// remain visibly identical; the fixture is intentionally integration-sized.
#[allow(clippy::too_many_lines)]
// Setup, lock observation, and result assertions are intentionally kept in
// one case so valid and invalid captures use the same ordering fixture.
#[allow(clippy::cognitive_complexity)]
async fn run_ordering_case(database_url: &str, ui_manifest: &str, valid: bool) {
    let admin = PgPoolOptions::new()
        .max_connections(8)
        .connect(database_url)
        .await
        .expect("connect ordering test database");
    let temporary = tempfile::tempdir().expect("ordering test temporary root");
    let storage = Arc::new(
        GitStorage::initialize(temporary.path().join("repositories"))
            .await
            .expect("initialize ordering Git storage"),
    );
    let setup_service = PgForgeRepository::new(admin.clone(), Arc::clone(&storage));
    setup_service.initialize().await.expect("apply migrations");

    let organization = OrganizationId::new();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization.as_uuid())
        .bind(format!("receive-ordering-{organization}"))
        .execute(&admin)
        .await
        .expect("seed organization");
    let project = setup_service
        .create_project_trusted(organization, "receive-ordering-project")
        .await
        .expect("seed project");
    let repository = setup_service
        .create_repository_trusted(&CreateRepository {
            project_id: project.id,
            name: String::from("repository"),
            default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("seed repository");
    seed_catalog_images(&admin, repository.id.as_uuid()).await;

    let owner = UserId::new();
    seed_permissions(
        &admin,
        owner,
        organization,
        project.id.as_uuid(),
        repository.id.as_uuid(),
    )
    .await;
    let identity = AuthenticatedIdentity::new(
        owner,
        "https://receive-manual-ordering.example",
        format!("receive-manual-{owner}"),
        json!({"email_verified": true}),
        RequestId::new(),
    );
    let config = CONFIG_TEMPLATE
        .replace(
            "__BUILD_IMAGE__",
            &format!("ordering-build-{}", repository.id.as_uuid().simple()),
        )
        .replace(
            "__RUNTIME_IMAGE__",
            &format!("ordering-runtime-{}", repository.id.as_uuid().simple()),
        );
    let (commit, update) = commit_source(&temporary, &repository, &config, ui_manifest).await;
    let parsed = agent_config::parse(config.as_bytes());
    let parsed_config = parsed.config.expect("valid agent configuration");
    let normalized_hash = decode_hash(
        parsed
            .normalized_hash
            .expect("normalized configuration hash")
            .as_str(),
    );
    let base_hash = agent_config::build_identity::base_build_definition_hash(
        parsed_config.build.as_ref().expect("build declaration"),
    )
    .expect("base build hash");

    let receive_name = format!(
        "receive-ordering-receive-{}",
        repository.id.as_uuid().simple()
    );
    let manual_name = format!(
        "receive-ordering-manual-{}",
        repository.id.as_uuid().simple()
    );
    let receive_pool = named_app_pool(database_url, &receive_name).await;
    let manual_pool = named_app_pool(database_url, &manual_name).await;
    let receive_service = PgForgeRepository::new(receive_pool.clone(), Arc::clone(&storage))
        .with_authorizer(Arc::new(PostgresMelangeAuthorizer));
    let application = BuildApplication::new(manual_pool.clone());

    let barrier_receive_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, principal, status, accepted_at, created_at)
         VALUES ($1, $2, 'ordering-barrier', 'accepted', now(), now())",
    )
    .bind(barrier_receive_id)
    .bind(repository.id.as_uuid())
    .execute(&admin)
    .await
    .expect("seed lock barrier receive");
    sqlx::query(
        "INSERT INTO git_refs
         (repository_id, git_ref, commit_sha, updated_by_receive_id)
         VALUES ($1, 'refs/heads/main', repeat('f', 40), $2)",
    )
    .bind(repository.id.as_uuid())
    .bind(barrier_receive_id)
    .execute(&admin)
    .await
    .expect("seed lock barrier ref");
    let mut barrier = admin.begin().await.expect("begin lock barrier");
    sqlx::query("SELECT set_config('application_name', $1, false)")
        .bind(format!(
            "receive-ordering-holder-{}",
            repository.id.as_uuid().simple()
        ))
        .execute(&mut *barrier)
        .await
        .expect("name lock barrier");
    let barrier_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *barrier)
        .await
        .expect("lock barrier pid");
    sqlx::query("SELECT git_ref FROM git_refs WHERE repository_id = $1 FOR UPDATE")
        .bind(repository.id.as_uuid())
        .execute(&mut *barrier)
        .await
        .expect("hold ref lock barrier");

    let receive_repository = repository.clone();
    let receive_identity = identity.clone();
    let receive_update = update.clone();
    let receive_task = tokio::spawn(async move {
        receive_service
            .accept_receive_as(
                &receive_repository,
                ReceiveId::new(),
                "receive-ordering",
                Some(&receive_identity),
                std::slice::from_ref(&receive_update),
            )
            .await
    });
    let receive_pid = wait_for_lock(&admin, &receive_name, "INSERT INTO git_refs").await;
    assert_blocked_by(&admin, receive_pid, barrier_pid).await;

    let manual_identity = identity.clone();
    let manual_request = RequestBuild {
        repository_id: repository.id.as_uuid(),
        source_commit: commit.as_str().to_owned(),
        build_definition_hash: base_hash,
        configuration_hash: normalized_hash,
    };
    let manual_task = tokio::spawn(async move {
        application
            .request_build(&manual_identity, manual_request)
            .await
    });
    let manual_pid = wait_for_lock(&admin, &manual_name, "FROM repositories").await;
    assert_blocked_by(&admin, manual_pid, receive_pid).await;
    barrier
        .commit()
        .await
        .expect("release receive lock barrier");

    let received = timeout(Duration::from_secs(10), receive_task)
        .await
        .expect("receive completion")
        .expect("receive task")
        .expect("accepted receive");
    let manual = timeout(Duration::from_secs(10), manual_task)
        .await
        .expect("manual request completion")
        .expect("manual request task");

    if valid {
        assert_eq!(received.build_requests.len(), 1);
        let received_build_id = received.build_requests[0].as_uuid();
        let manual = manual.expect("manual request observes valid capture");
        assert_eq!(manual.id, received_build_id);
        let stored_hash: Vec<u8> =
            sqlx::query_scalar("SELECT build_definition_hash FROM build_requests WHERE id = $1")
                .bind(received_build_id)
                .fetch_one(&admin)
                .await
                .expect("stored receive build hash");
        let ui_hash: Vec<u8> = sqlx::query_scalar(
            "SELECT normalized_ui_hash
             FROM ui_source_manifest_revisions
             WHERE repository_id = $1 AND source_commit = $2",
        )
        .bind(repository.id.as_uuid())
        .bind(commit.as_str())
        .fetch_one(&admin)
        .await
        .expect("stored valid UI hash");
        let ui_hash: [u8; 32] = ui_hash.try_into().expect("valid UI hash width");
        let expected_hash =
            agent_config::build_identity::ui_build_definition_hash(base_hash, ui_hash, None);
        assert_ne!(expected_hash, base_hash);
        assert_eq!(stored_hash, expected_hash.to_vec());
        let link_count: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM build_request_ui_source_manifests
             WHERE build_request_id = $1 AND source_status = 'valid'",
        )
        .bind(received_build_id)
        .fetch_one(&admin)
        .await
        .expect("valid UI build link");
        assert_eq!(link_count, 1);
        let event_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM outbox
             WHERE subject = 'hephaestus.build.requested.v1' AND aggregate_id = $1",
        )
        .bind(received_build_id)
        .fetch_one(&admin)
        .await
        .expect("valid build event count");
        assert_eq!(event_count, 1);
    } else {
        assert_eq!(received.invalid_configurations, 0);
        assert!(received.build_requests.is_empty());
        assert!(matches!(manual, Err(BuildError::FailedPrecondition)));
        let build_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM build_requests
             WHERE repository_id = $1 AND source_commit = $2",
        )
        .bind(repository.id.as_uuid())
        .bind(commit.as_str())
        .fetch_one(&admin)
        .await
        .expect("invalid build count");
        assert_eq!(build_count, 0);
        let event_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM outbox
             WHERE subject = 'hephaestus.build.requested.v1'
               AND payload->>'source_commit' = $1",
        )
        .bind(commit.as_str())
        .fetch_one(&admin)
        .await
        .expect("invalid build event count");
        assert_eq!(event_count, 0);
    }

    println!(
        "REAL_RECEIVE_MANUAL_UI_ORDERING=1 valid={valid} holder_pid={barrier_pid} receive_pid={receive_pid} manual_pid={manual_pid} received_builds={}",
        received.build_requests.len()
    );

    // UI captures and their build links are immutable evidence under migration
    // 0082. The disposable test-database runner owns final fixture disposal.
    drop(setup_service);
    receive_pool.close().await;
    manual_pool.close().await;
    admin.close().await;
}

async fn named_app_pool(database_url: &str, application_name: &str) -> PgPool {
    let application_name = application_name.to_owned();
    PgPoolOptions::new()
        .max_connections(1)
        .after_connect(move |connection, _metadata| {
            let application_name = application_name.clone();
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_app")
                    .execute(&mut *connection)
                    .await?;
                sqlx::query("SELECT set_config('application_name', $1, false)")
                    .bind(application_name)
                    .execute(&mut *connection)
                    .await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect named application pool")
}

async fn wait_for_lock(pool: &PgPool, application_name: &str, query_fragment: &str) -> i32 {
    let pattern = format!("%{query_fragment}%");
    timeout(Duration::from_secs(10), async {
        loop {
            let row: Option<(i32,)> = sqlx::query_as(
                "SELECT pid
                 FROM pg_stat_activity
                 WHERE application_name = $1 AND state = 'active'
                   AND wait_event_type = 'Lock' AND query LIKE $2
                 LIMIT 1",
            )
            .bind(application_name)
            .bind(&pattern)
            .fetch_optional(pool)
            .await
            .expect("inspect ordering lock wait");
            if let Some((pid,)) = row {
                return pid;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("ordering operation reached expected lock wait")
}

async fn assert_blocked_by(pool: &PgPool, waiting_pid: i32, blocker_pid: i32) {
    let blockers: Vec<i32> = sqlx::query_scalar(
        "SELECT unnest(pg_blocking_pids(pid))
         FROM pg_stat_activity WHERE pid = $1",
    )
    .bind(waiting_pid)
    .fetch_all(pool)
    .await
    .expect("inspect ordering blocker");
    assert!(
        blockers.contains(&blocker_pid),
        "PID {waiting_pid} was not blocked by {blocker_pid}: {blockers:?}"
    );
}

async fn seed_permissions(
    pool: &PgPool,
    owner: UserId,
    organization: OrganizationId,
    project: Uuid,
    repository: Uuid,
) {
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'receive-ordering-owner')")
        .bind(owner.as_uuid())
        .execute(pool)
        .await
        .expect("seed ordering owner");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization.as_uuid())
    .bind(owner.as_uuid())
    .execute(pool)
    .await
    .expect("seed ordering organization membership");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project)
        .bind(owner.as_uuid())
        .execute(pool)
        .await
        .expect("seed ordering project maintainer");
    sqlx::query("INSERT INTO repository_managers (repository_id, user_id) VALUES ($1, $2)")
        .bind(repository)
        .bind(owner.as_uuid())
        .execute(pool)
        .await
        .expect("seed ordering repository manager");
}

async fn seed_catalog_images(pool: &PgPool, repository: Uuid) {
    for (kind, byte) in [("build", b'a'), ("runtime", b'b')] {
        let key = format!("ordering-{kind}-{}", repository.simple());
        let reference = format!(
            "ordering-{kind}-{}@sha256:{}",
            repository.simple(),
            char::from(byte).to_string().repeat(64)
        );
        sqlx::query(
            "INSERT INTO oci_images
             (id, key, display_name, image_reference, toolchains, architectures,
              availability_state, provenance, platform_policy_version, role)
             VALUES ($1, $2, $3, $4, '[]', ARRAY['x86_64'], 'available',
                     '{}', 'test/v1', 'execution')",
        )
        .bind(Uuid::new_v4())
        .bind(key)
        .bind(format!("Ordering {kind} image"))
        .bind(reference)
        .execute(pool)
        .await
        .expect("seed ordering image");
    }
}

async fn commit_source(
    temporary: &TempDir,
    repository: &Repository,
    config: &str,
    ui_manifest: &str,
) -> (CommitSha, RefUpdate) {
    let work = temporary.path().join("work");
    tokio::fs::create_dir(&work)
        .await
        .expect("ordering work directory");
    git(&work, &["init", "--initial-branch=main"]).await;
    git(&work, &["config", "user.name", "Hephaestus Test"]).await;
    git(
        &work,
        &["config", "user.email", "hephaestus@example.invalid"],
    )
    .await;
    tokio::fs::write(work.join("agent.toml"), config)
        .await
        .expect("ordering agent configuration");
    tokio::fs::write(work.join("heph.ui.toml"), ui_manifest)
        .await
        .expect("ordering UI manifest");
    git(&work, &["add", "."]).await;
    git(&work, &["commit", "-m", "ordering fixture"]).await;
    let commit =
        CommitSha::parse(git_output(&work, &["rev-parse", "HEAD"]).await).expect("ordering commit");
    let bare = temporary
        .path()
        .join("repositories")
        .join(format!("{}.git", repository.id));
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("ordering bare path"),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    (
        commit.clone(),
        RefUpdate {
            git_ref: GitRef::parse("refs/heads/main").expect("ordering ref"),
            old_commit: None,
            new_commit: Some(commit),
        },
    )
}

async fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run ordering Git command");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run ordering Git command");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("ordering Git output")
        .trim()
        .to_owned()
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
