use super::*;
use hephaestus_app::RunningHephaestus;
use sqlx::PgPool;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;
use uuid::Uuid;
pub(crate) async fn initialize_session_checkout(
    checkout: &Path,
    source_root: &Path,
    repository: Uuid,
    git_token: &str,
    running: &RunningHephaestus,
    human_id: &str,
) {
    let initializer = r"
from pathlib import Path
import sys
from git_adapter import LocalGitSession
LocalGitSession.initialize(Path(sys.argv[1]), sys.argv[2], human_ids=(sys.argv[3],))
";
    let output = Command::new("python3")
        .arg("-c")
        .arg(initializer)
        .arg(checkout)
        .arg(SESSION_ID)
        .arg(human_id)
        .env("PYTHONPATH", source_root)
        .stdin(Stdio::null())
        .output()
        .await
        .expect("initialize session checkout");
    assert!(
        output.status.success(),
        "session initialization failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let remote = format!("http://{}/{}", running.http_addr(), repository);
    git(checkout, &["remote", "add", "origin", &remote]).await;
    authenticated_git(
        checkout,
        git_token,
        &["push", "origin", "HEAD:refs/heads/main"],
    )
    .await;
}

pub(crate) async fn append_human_and_push(
    checkout: &Path,
    source_root: &Path,
    git_token: &str,
    _running: &RunningHephaestus,
    user_id: &str,
    baseline: &str,
    human_record_id: &str,
) {
    let appender = r#"
from pathlib import Path
import sys
from git_adapter import LocalGitSession
from protocol import Record, TextContent, utc_now
session = LocalGitSession.open(Path(sys.argv[1]))
actor = "user:" + sys.argv[4]
session.append_human(Record(sys.argv[3], "user_message", actor, "human", actor, utc_now(), content=TextContent("hello session")), sys.argv[2])
"#;
    let output = Command::new("python3")
        .arg("-c")
        .arg(appender)
        .arg(checkout)
        .arg(baseline)
        .arg(human_record_id)
        .arg(user_id)
        .env("PYTHONPATH", source_root)
        .stdin(Stdio::null())
        .output()
        .await
        .expect("append session human record");
    assert!(
        output.status.success(),
        "session human append failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    authenticated_git(
        checkout,
        git_token,
        &["push", "origin", "HEAD:refs/heads/main"],
    )
    .await;
}
