//! Rust workspace checks and opt-in integration runners.

use crate::{
    context::DevContext,
    process::{DevError, Result, run as run_process},
};
use std::{env, path::Path, process::Command};

use super::{
    quality,
    support::{cargo, phase},
};

pub(super) fn run(context: &DevContext) -> Result<()> {
    let root = &context.repository_root;
    let browser_session_rpc = env::var("REAL_APP_BROWSER_SESSION_RPC").as_deref() == Ok("1");
    let ui_installation_rpc_enabled = env::var("REAL_UI_INSTALLATION_RPC").as_deref() == Ok("1");
    let artifact_deadline_rpc_enabled = env::var("REAL_APP_ARTIFACT_RPC").as_deref() == Ok("1");
    phase("Rust formatting");
    cargo(root, &["fmt", "--all", "--", "--check"])?;
    phase("Rust Clippy");
    cargo(
        root,
        &["clippy", "--workspace", "--all-targets", "--all-features"],
    )?;
    phase("Rust tests");
    workspace_tests(
        root,
        browser_session_rpc,
        ui_installation_rpc_enabled,
        artifact_deadline_rpc_enabled,
    )?;
    browser_session_lifecycle(context, browser_session_rpc)?;
    ui_installation_rpc(context, ui_installation_rpc_enabled)?;
    artifact_deadline_rpc(context, artifact_deadline_rpc_enabled)?;
    phase("Rust documentation");
    cargo(root, &["doc", "--workspace", "--all-features", "--no-deps"])?;
    quality::cooking_service(context)
}

fn workspace_tests(
    root: &Path,
    browser_session_rpc: bool,
    ui_installation_rpc: bool,
    artifact_deadline_rpc: bool,
) -> Result<()> {
    let mut command = Command::new("cargo");
    command
        .args(["test", "--workspace", "--all-features"])
        .current_dir(root);
    if browser_session_rpc {
        // Run the opt-in proof after this shared pass against its disposable
        // database instead of the caller's potentially contaminated database.
        command.env_remove("REAL_APP_BROWSER_SESSION_RPC");
    }
    if ui_installation_rpc {
        // Run the opt-in proof after this shared pass against its disposable
        // database instead of the caller's potentially contaminated database.
        command.env_remove("REAL_UI_INSTALLATION_RPC");
    }
    if artifact_deadline_rpc {
        // Run the opt-in proof after this shared pass against its disposable
        // database instead of the caller's potentially contaminated database.
        command.env_remove("REAL_APP_ARTIFACT_RPC");
    }
    run_process(&mut command)
}

fn browser_session_lifecycle(context: &DevContext, enabled: bool) -> Result<()> {
    if !enabled {
        return Ok(());
    }
    let script = context
        .repository_root
        .join("scripts/test-browser-session-lifecycle.sh");
    if !script.is_file() {
        return Err(DevError::Invalid(format!(
            "browser-session lifecycle runner is missing at {}",
            script.display()
        )));
    }
    phase("Browser-session RPC lifecycle integration (isolated database)");
    let mut command = Command::new(script);
    command.current_dir(&context.repository_root);
    if env::var("HEPHAESTUS_BROWSER_SESSION_POSTGRES_MODE").as_deref() == Ok("container") {
        command.env(
            "HEPHAESTUS_POSTGRES_CONTAINER",
            env::var("HEPHAESTUS_POSTGRES_CONTAINER")
                .unwrap_or_else(|_| context.postgres_container()),
        );
    }
    run_process(&mut command)
}

fn ui_installation_rpc(context: &DevContext, enabled: bool) -> Result<()> {
    if !enabled {
        return Ok(());
    }
    let script = context
        .repository_root
        .join("scripts/test-ui-installation-rpc.sh");
    if !script.is_file() {
        return Err(DevError::Invalid(format!(
            "UI installation RPC runner is missing at {}",
            script.display()
        )));
    }
    phase("UI installation RPC integration (isolated database)");
    let mut command = Command::new(script);
    command.current_dir(&context.repository_root);
    if env::var("HEPHAESTUS_UI_RPC_POSTGRES_MODE").as_deref() == Ok("container") {
        command.env(
            "HEPHAESTUS_POSTGRES_CONTAINER",
            env::var("HEPHAESTUS_POSTGRES_CONTAINER")
                .unwrap_or_else(|_| context.postgres_container()),
        );
    }
    run_process(&mut command)
}

fn artifact_deadline_rpc(context: &DevContext, enabled: bool) -> Result<()> {
    if !enabled {
        return Ok(());
    }
    let script = context
        .repository_root
        .join("scripts/test-artifact-deadline-rpc.sh");
    if !script.is_file() {
        return Err(DevError::Invalid(format!(
            "artifact RPC deadline runner is missing at {}",
            script.display()
        )));
    }
    phase("Artifact RPC deadline integration (isolated database)");
    let mut command = Command::new(script);
    command.current_dir(&context.repository_root);
    if env::var("HEPHAESTUS_ARTIFACT_RPC_POSTGRES_MODE").as_deref() == Ok("container") {
        command.env(
            "HEPHAESTUS_POSTGRES_CONTAINER",
            env::var("HEPHAESTUS_POSTGRES_CONTAINER")
                .unwrap_or_else(|_| context.postgres_container()),
        );
    }
    run_process(&mut command)
}
