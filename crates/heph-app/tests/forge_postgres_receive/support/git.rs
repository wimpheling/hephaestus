use super::*;

pub async fn commit_and_update(
    temporary: &tempfile::TempDir,
    repository: &Repository,
    config: &str,
) -> (CommitSha, RefUpdate) {
    commit_and_update_files(temporary, repository, Some(config), None).await
}

pub async fn commit_and_update_with_ui(
    temporary: &tempfile::TempDir,
    repository: &Repository,
    config: &str,
    ui_manifest: &str,
) -> (CommitSha, RefUpdate) {
    commit_and_update_files(temporary, repository, Some(config), Some(ui_manifest)).await
}

pub async fn commit_and_update_files(
    temporary: &tempfile::TempDir,
    repository: &Repository,
    config: Option<&str>,
    ui_manifest: Option<&str>,
) -> (CommitSha, RefUpdate) {
    let work = temporary.path().join("work");
    tokio::fs::create_dir(&work).await.expect("work directory");
    git(&work, &["init", "--initial-branch=main"]).await;
    git(&work, &["config", "user.name", "Hephaestus Test"]).await;
    git(
        &work,
        &["config", "user.email", "hephaestus@example.invalid"],
    )
    .await;
    if let Some(config) = config {
        tokio::fs::write(work.join("agent.toml"), config)
            .await
            .expect("agent configuration");
    }
    if let Some(ui_manifest) = ui_manifest {
        tokio::fs::write(work.join("heph.ui.toml"), ui_manifest)
            .await
            .expect("UI manifest");
    }
    git(&work, &["add", "."]).await;
    git(&work, &["commit", "-m", "agent config"]).await;
    let commit =
        CommitSha::parse(git_output(&work, &["rev-parse", "HEAD"]).await).expect("commit ID");
    let bare = temporary
        .path()
        .join("repositories")
        .join(format!("{}.git", repository.id));
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("UTF-8 bare path"),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    (
        commit.clone(),
        RefUpdate {
            git_ref: GitRef::parse("refs/heads/main").expect("updated ref"),
            old_commit: None,
            new_commit: Some(commit),
        },
    )
}

pub async fn cleanup(pool: &PgPool, repository: Repository) {
    let retains_instance_history: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM agent_attachments WHERE repository_id = $1
         )",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("instance provenance retention check");
    if retains_instance_history {
        // Reusable attachment/release provenance is deliberately permanent;
        // this random fixture cannot be deleted without violating that model.
        return;
    }
    sqlx::query(
        "DELETE FROM outbox WHERE aggregate_type = 'forge'
         AND (
           aggregate_id IN (SELECT id FROM run_requests WHERE repository_id = $1)
           OR aggregate_id IN (SELECT id FROM agent_config_revisions WHERE repository_id = $1)
           OR aggregate_id IN (SELECT id FROM git_receives WHERE repository_id = $1)
         )",
    )
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("delete forge outbox");
    sqlx::query("DELETE FROM run_requests WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run requests");
    sqlx::query("DELETE FROM agent_config_revisions WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete config revisions");
    sqlx::query("DELETE FROM git_refs WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete current refs");
    sqlx::query(
        "DELETE FROM git_ref_updates
         WHERE receive_id IN (SELECT id FROM git_receives WHERE repository_id = $1)",
    )
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("delete ref updates");
    sqlx::query("DELETE FROM git_receives WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete receives");
    sqlx::query("DELETE FROM repositories WHERE id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete repository");
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(repository.project_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete project");
}

pub async fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub async fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 Git output")
        .trim()
        .to_owned()
}
