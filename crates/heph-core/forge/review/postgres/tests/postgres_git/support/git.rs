use review_domain::ControlKind;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{path::Path, process::Command};

pub fn run_git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {:?}: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn git_text(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args(arguments)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {:?}: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

pub fn git_text_with_identity(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .env("GIT_AUTHOR_NAME", "Concurrent Reviewer")
        .env("GIT_AUTHOR_EMAIL", "concurrent@example.invalid")
        .env("GIT_COMMITTER_NAME", "Concurrent Reviewer")
        .env("GIT_COMMITTER_EMAIL", "concurrent@example.invalid")
        .args(arguments)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {:?}: {}",
        arguments,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

pub const fn kind_name(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::CancelRun => "cancel_run",
        ControlKind::RetryRun => "retry_run",
        ControlKind::ApproveResult => "approve_result",
        ControlKind::RejectResult => "reject_result",
    }
}

pub async fn pool() -> Option<PgPool> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    Some(
        PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .expect("PostgreSQL connection"),
    )
}
