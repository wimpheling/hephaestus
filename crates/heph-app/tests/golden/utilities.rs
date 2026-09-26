use super::*;

/// Emits bounded, payload-free state when the production golden run times out.
/// This is intentionally test-only: it helps distinguish an un-dispatched
/// command from a persisted guest failure without exposing guest logs or
/// provider error strings (which could contain paths or secret material).
// The diagnostic deliberately keeps each bounded status query together so a
// timeout report is emitted atomically from this test-only helper.
#[allow(clippy::too_many_lines, clippy::type_complexity)]
pub async fn diagnose_golden_timeout(
    pool: &sqlx::PgPool,
    repository_id: uuid::Uuid,
    run_id: runtime_types::RunId,
) {
    let run = sqlx::query(
        "SELECT state, outcome, exit_code, exit_signal,
                vm_id IS NOT NULL AS has_vm
           FROM runs WHERE id = $1",
    )
    .bind(run_id.as_uuid())
    .fetch_optional(pool)
    .await;
    match run {
        Ok(Some(row)) => {
            let state: String = row.get("state");
            let outcome: Option<String> = row.get("outcome");
            let exit_code: Option<i32> = row.get("exit_code");
            let exit_signal: Option<i32> = row.get("exit_signal");
            let has_vm: bool = row.get("has_vm");
            eprintln!(
                "golden timeout: run state={state} outcome={outcome:?} exit_code={exit_code:?} exit_signal={exit_signal:?} has_vm={has_vm}"
            );
        }
        Ok(None) => eprintln!("golden timeout: run row missing"),
        Err(error) => eprintln!("golden timeout: run query failed: {error}"),
    }

    let request = sqlx::query(
        "SELECT request_kind, dispatch_state
           FROM run_requests WHERE run_id = $1",
    )
    .bind(run_id.as_uuid())
    .fetch_optional(pool)
    .await;
    match request {
        Ok(Some(row)) => {
            let kind: String = row.get("request_kind");
            let dispatch_state: String = row.get("dispatch_state");
            eprintln!("golden timeout: run request kind={kind} dispatch_state={dispatch_state}");
        }
        Ok(None) => eprintln!("golden timeout: run request row missing"),
        Err(error) => eprintln!("golden timeout: run request query failed: {error}"),
    }

    match sqlx::query_scalar::<_, String>(
        "SELECT event_type FROM run_events WHERE run_id = $1 ORDER BY sequence",
    )
    .bind(run_id.as_uuid())
    .fetch_all(pool)
    .await
    {
        Ok(events) => eprintln!("golden timeout: run events={events:?}"),
        Err(error) => eprintln!("golden timeout: run events query failed: {error}"),
    }

    match sqlx::query(
        "SELECT subject, published_at IS NOT NULL AS published
           FROM outbox
          WHERE aggregate_id = $1
          ORDER BY occurred_at, id",
    )
    .bind(run_id.as_uuid())
    .fetch_all(pool)
    .await
    {
        Ok(rows) => {
            let entries: Vec<(String, bool)> = rows
                .into_iter()
                .map(|row| (row.get("subject"), row.get("published")))
                .collect();
            eprintln!("golden timeout: run outbox={entries:?}");
        }
        Err(error) => eprintln!("golden timeout: run outbox query failed: {error}"),
    }

    match sqlx::query(
        "SELECT request.state AS request_state,
                execution.state AS execution_state,
                execution.exit_code, execution.exit_signal,
                execution.failure_code
           FROM build_requests AS request
           LEFT JOIN build_executions AS execution
             ON execution.build_request_id = request.id
          WHERE request.repository_id = $1
          ORDER BY request.created_at DESC, request.id DESC
          LIMIT 3",
    )
    .bind(repository_id)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => {
            let entries: Vec<(
                String,
                Option<String>,
                Option<i32>,
                Option<i32>,
                Option<String>,
            )> = rows
                .into_iter()
                .map(|row| {
                    (
                        row.get("request_state"),
                        row.get("execution_state"),
                        row.get("exit_code"),
                        row.get("exit_signal"),
                        row.get("failure_code"),
                    )
                })
                .collect();
            eprintln!("golden timeout: recent builds={entries:?}");
        }
        Err(error) => eprintln!("golden timeout: build query failed: {error}"),
    }
}

pub async fn git_backend() -> PathBuf {
    let exec_path = git_output(Path::new("."), &["--exec-path"]).await;
    PathBuf::from(exec_path).join("git-http-backend")
}

pub async fn git_binary() -> PathBuf {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .await
        .expect("resolve Git binary");
    assert!(output.status.success());
    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("UTF-8 Git path")
            .trim(),
    )
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
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub async fn authenticated_git(directory: &Path, token: &str, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Bearer {token}"),
        )
        .output()
        .await
        .expect("run authenticated Git");
    assert!(
        output.status.success(),
        "authenticated Git failed: {} {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
}

pub async fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run Git");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("UTF-8 Git output")
        .trim()
        .to_owned()
}

pub async fn git_output_bare(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repository.display()))
        .args(arguments)
        .output()
        .await
        .expect("run bare Git");
    assert!(
        output.status.success(),
        "bare Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 bare Git output")
        .trim()
        .to_owned()
}

pub fn mkfs_ext4() -> PathBuf {
    if let Some(path) = env::var_os("HEPHAESTUS_MKFS_EXT4") {
        return PathBuf::from(path);
    }
    ["/usr/sbin/mkfs.ext4", "/usr/bin/mkfs.ext4"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .expect("mkfs.ext4 must be installed for the golden volume fixture")
}

pub async fn cleanup_streams(nats_url: &str) {
    let Ok(client) = async_nats::connect(nats_url).await else {
        return;
    };
    let context = async_nats::jetstream::new(client);
    for stream in [
        "HEPH_RUN_COMMANDS",
        "HEPH_RUN_EVENTS",
        "HEPHAESTUS_GIT_EVENTS",
        "HEPHAESTUS_RELEASE_EVENTS",
        "HEPHAESTUS_PRODUCT_EVENTS",
    ] {
        drop(context.delete_stream(stream).await);
    }
}
