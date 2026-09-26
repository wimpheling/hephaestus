//! Shared command helpers for repository checks.

use crate::{
    context::{DevContext, ELIXIR_IMAGE},
    process::{DevError, Result, run as run_process},
};
use std::{path::Path, process::Command};

pub(super) fn cargo(root: &Path, arguments: &[&str]) -> Result<()> {
    run_process(Command::new("cargo").args(arguments).current_dir(root))
}

pub(super) fn cargo_manifest(
    root: &Path,
    manifest: &Path,
    subcommand: &str,
    arguments: &[&str],
) -> Result<()> {
    run_process(
        Command::new("cargo")
            .arg(subcommand)
            .arg("--manifest-path")
            .arg(manifest)
            .args(arguments)
            .current_dir(root),
    )
}

pub(super) fn require_mix_project(web: &Path) -> Result<()> {
    if web.join("mix.exs").is_file() {
        Ok(())
    } else {
        Err(DevError::Invalid(format!(
            "Phoenix project is missing at {}",
            web.display()
        )))
    }
}

pub(super) fn web_mix_command(context: &DevContext, checks: &str) -> Command {
    let mount = format!("{}:/workspace:z", context.repository_root.display());
    let script = format!(
        "apt-get update -qq && apt-get install -y -qq --no-install-recommends git ca-certificates >/dev/null && rm -rf /var/lib/apt/lists/* && mix local.hex --force >/dev/null && mix local.rebar --force >/dev/null && mix deps.get && {checks}"
    );
    let mut command = Command::new("podman");
    command
        .args([
            "run",
            "--rm",
            "--network",
            "host",
            "--volume",
            &mount,
            "--workdir",
            "/workspace/web",
            "--env",
            "MIX_ENV=test",
            ELIXIR_IMAGE,
            "sh",
            "-lc",
            &script,
        ])
        .current_dir(&context.repository_root);
    command
}

pub(super) fn phase(name: &str) {
    println!("\n== {name} ==");
}
