use super::*;
use super::{SessionBrokerFixture, denial_probe_enabled, fork_e2e_enabled, restart_e2e_enabled};
use hephaestus_app::RunningHephaestus;
use identity_domain::AuthenticatedIdentity;
use rpc_proto::messages::hephaestus::common::v1::RequestContext;
use sqlx::PgPool;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;
use uuid::Uuid;
pub(crate) async fn grant_session_capability(
    pool: &PgPool,
    project: ProjectId,
    identity: &AuthenticatedIdentity,
) {
    // Browser OIDC resolves golden-subject to this same identity; the explicit
    // project grant authorizes the user-selected repository capability.
    sqlx::query(
        "INSERT INTO project_capability_granters (project_id, user_id, created_by)
         VALUES ($1, $2, $2)
         ON CONFLICT (project_id, user_id) DO NOTHING",
    )
    .bind(project.as_uuid())
    .bind(identity.user_id.as_uuid())
    .execute(pool)
    .await
    .expect("session capability delegation role");
}

pub(crate) enum SessionChatBrowserMode {
    New,
    Existing {
        repository_id: Uuid,
        installation_id: Uuid,
        generation_id: Uuid,
        actor_id: Uuid,
    },
    Concurrent {
        repository_id: Uuid,
        installation_id: Uuid,
        generation_id: Uuid,
        actor_id: Uuid,
        initial_transcript_count: usize,
        initial_agent_count: usize,
    },
    Fork {
        repository_id: Uuid,
        installation_id: Uuid,
        generation_id: Uuid,
        actor_id: Uuid,
        initial_transcript_count: usize,
        initial_agent_count: usize,
    },
}

#[allow(clippy::too_many_lines)] // Browser setup and selector validation stay one bounded fixture boundary.
pub(crate) async fn run_session_chat_browser(
    database_url: &str,
    running: &RunningHephaestus,
    project: ProjectId,
    release_agent_id: Uuid,
    model_import_id: Uuid,
    mode: SessionChatBrowserMode,
) {
    let browser_phase = match &mode {
        SessionChatBrowserMode::New => "initial",
        SessionChatBrowserMode::Existing { .. } => "recovery",
        SessionChatBrowserMode::Concurrent { .. } => "concurrency",
        SessionChatBrowserMode::Fork { .. } => "fork",
    };
    let fixture_output = PathBuf::from(
        std::env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT")
            .expect("session-chat browser fixture output path"),
    );
    let (fixture, browser_mode, browser_selector) = match mode {
        SessionChatBrowserMode::New => (
            serde_json::json!({
                "session_chat_new": {
                    "project_id": project,
                    "release_agent_id": release_agent_id,
                    "model_import_id": model_import_id,
                    "ui_path": "/session-chat/index.html",
                    "agent_response_text": MODEL_RESPONSE_TEXT
                }
            }),
            "session_chat_new",
            "cooking new session chat creates and opens a real Git-backed browser session",
        ),
        SessionChatBrowserMode::Existing {
            repository_id,
            installation_id,
            generation_id,
            actor_id,
        } => (
            serde_json::json!({
                "session_chat_ui": {
                    "project_id": project,
                    "repository_id": repository_id,
                    "installation_id": installation_id,
                    "generation_id": generation_id,
                    "actor_id": actor_id,
                    "ui_path": "/session-chat/index.html",
                    "agent_response_text": MODEL_RESPONSE_TEXT,
                    "initial_transcript_count": "4",
                    "initial_agent_count": "2"
                }
            }),
            "session_chat_ui",
            "cooking session-chat installed UI initializes and reconnects ordinary Git history",
        ),
        SessionChatBrowserMode::Concurrent {
            repository_id,
            installation_id,
            generation_id,
            actor_id,
            initial_transcript_count,
            initial_agent_count,
        } => (
            serde_json::json!({
                "session_chat_concurrent": {
                    "project_id": project,
                    "repository_id": repository_id,
                    "installation_id": installation_id,
                    "generation_id": generation_id,
                    "actor_id": actor_id,
                    "ui_path": "/session-chat/index.html",
                    "agent_response_text": MODEL_RESPONSE_TEXT,
                    "initial_transcript_count": initial_transcript_count.to_string(),
                    "initial_agent_count": initial_agent_count.to_string()
                }
            }),
            "session_chat_concurrent",
            "cooking concurrent session chat clients reconcile a stale Git push and preserve both turns",
        ),
        SessionChatBrowserMode::Fork {
            repository_id,
            installation_id,
            generation_id,
            actor_id,
            initial_transcript_count,
            initial_agent_count,
        } => (
            serde_json::json!({
                "session_chat_fork": {
                    "project_id": project,
                    "repository_id": repository_id,
                    "installation_id": installation_id,
                    "generation_id": generation_id,
                    "actor_id": actor_id,
                    "ui_path": "/session-chat/index.html",
                    "agent_response_text": MODEL_RESPONSE_TEXT,
                    "initial_transcript_count": initial_transcript_count.to_string(),
                    "initial_agent_count": initial_agent_count.to_string()
                }
            }),
            "session_chat_fork",
            "cooking forked session chat preserves inherited history and receives a fresh response",
        ),
    };
    let fixture_path = match browser_mode {
        "session_chat_new" => fixture_output,
        "session_chat_ui" => fixture_output.with_file_name(format!(
            "{}.recovery.json",
            fixture_output
                .file_name()
                .expect("session-chat fixture filename")
                .to_string_lossy()
        )),
        "session_chat_concurrent" => fixture_output.with_file_name(format!(
            "{}.concurrency.json",
            fixture_output
                .file_name()
                .expect("session-chat fixture filename")
                .to_string_lossy()
        )),
        "session_chat_fork" => fixture_output.with_file_name(format!(
            "{}.fork.json",
            fixture_output
                .file_name()
                .expect("session-chat fixture filename")
                .to_string_lossy()
        )),
        _ => unreachable!("session-chat browser fixture mode is allowlisted"),
    };
    eprintln!(
        "HEPH_SESSION_CHAT_BROWSER stage=fixture-selected mode={browser_mode} selector={browser_selector}"
    );
    tokio::fs::write(
        &fixture_path,
        serde_json::to_vec_pretty(&fixture).expect("session-chat browser fixture JSON"),
    )
    .await
    .expect("write session-chat browser fixture JSON");
    let control_dir = Path::new(&fixture_path)
        .parent()
        .expect("session-chat fixture parent")
        .join("installed-ui-control");
    match tokio::fs::create_dir(&control_dir).await {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = tokio::fs::symlink_metadata(&control_dir)
                .await
                .expect("read existing session-chat installed UI control directory");
            assert!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "session-chat installed UI control directory must remain a real directory"
            );
        }
        Err(error) => panic!("create session-chat installed UI control directory: {error}"),
    }
    std::fs::set_permissions(
        &control_dir,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("session-chat installed UI control directory mode");
    let issuer = std::env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER")
        .expect("session-chat browser OIDC issuer");
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/run-installed-ui-e2e.sh");
    let browser_timer = crate::WorkloadPhaseTimer::start(
        match browser_phase {
            "initial" => "browser-initial",
            "recovery" => "browser-recovery",
            "concurrency" => "browser-concurrency",
            "fork" => "browser-fork",
            _ => unreachable!("session-chat browser phase is allowlisted"),
        },
        crate::workload_phase_timing_from_environment(),
    );
    let status = Command::new(script)
        .env("HEPHAESTUS_E2E_COOKING_FIXTURE", &fixture_path)
        .env("HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL", database_url)
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT",
            running.http_addr().to_string(),
        )
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET",
            "golden-internal-command-token-with-sufficient-entropy",
        )
        .env("HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER", issuer)
        .env("HEPHAESTUS_E2E_COOKING_PHASE", browser_phase)
        .env("HEPHAESTUS_INSTALLED_UI_BROWSER_GREP", browser_selector)
        .env(
            "HEPHAESTUS_PLATFORM_HTTPS_ORIGIN",
            crate::installed_ui_platform_origin(),
        )
        .env("HEPHAESTUS_UI_NAMESPACE", crate::installed_ui_namespace())
        .env(
            "HEPHAESTUS_CADDY_TEST_CA_CERT",
            std::env::var("HEPHAESTUS_CADDY_TEST_CA_CERT")
                .expect("session-chat browser Caddy CA certificate"),
        )
        .status()
        .await
        .expect("run session-chat installed UI browser E2E");
    browser_timer.finish(status.success());
    assert!(
        status.success(),
        "session-chat browser E2E failed: {status}"
    );
    eprintln!(
        "HEPH_SESSION_CHAT_BROWSER stage=playwright-passed mode={browser_mode} selector={browser_selector}"
    );
}
