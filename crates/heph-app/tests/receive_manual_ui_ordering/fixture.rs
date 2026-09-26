use forge_domain::{CommitSha, GitRef, OrganizationId, RefUpdate, Repository};
use identity_domain::UserId;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{path::Path, time::Duration};
use tempfile::TempDir;
use tokio::{process::Command, time::timeout};
use uuid::Uuid;

pub async fn named_app_pool(database_url: &str, application_name: &str) -> PgPool {
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

pub async fn wait_for_lock(pool: &PgPool, application_name: &str, query_fragment: &str) -> i32 {
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

pub async fn assert_blocked_by(pool: &PgPool, waiting_pid: i32, blocker_pid: i32) {
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

pub async fn seed_permissions(
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

pub async fn seed_catalog_images(pool: &PgPool, repository: Uuid) {
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

pub async fn commit_source(
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

pub fn decode_hash(value: &str) -> [u8; 32] {
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
