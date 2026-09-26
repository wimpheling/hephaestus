//! Full quality-gate orchestration and web checks.

use crate::{
    context::DevContext,
    process::{DevError, Result, run as run_process},
};
use std::process::Command;

use super::{
    QUALITY_FAMILIES, architecture, protobuf, rust,
    support::{cargo_manifest, phase, require_mix_project, web_mix_command},
};

const UI_CHECKS: &str = "mix hephaestus.architecture --family ui && mix test test/mix/tasks/hephaestus_architecture_test.exs test/hephaestus_web_web/components test/hephaestus_web_web/design_system";
const COOKING_SERVICE_MANIFEST: &str = "examples/cooking/cooking-service/Cargo.toml";
const RELEASE_UI_KIT_DIRECTORY: &str = "web/assets/release_ui_kit";

pub(super) fn cooking_service(context: &DevContext) -> Result<()> {
    let root = &context.repository_root;
    let manifest = root.join(COOKING_SERVICE_MANIFEST);
    if !manifest.is_file() {
        return Err(DevError::Invalid(format!(
            "cooking service manifest is missing at {}",
            manifest.display()
        )));
    }

    phase("Cooking service formatting");
    cargo_manifest(root, &manifest, "fmt", &["--", "--check"])?;
    phase("Cooking service Clippy");
    cargo_manifest(
        root,
        &manifest,
        "clippy",
        &[
            "--locked",
            "--offline",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    phase("Cooking service tests");
    cargo_manifest(
        root,
        &manifest,
        "test",
        &["--locked", "--offline", "--all-features"],
    )?;
    phase("Cooking service documentation");
    cargo_manifest(
        root,
        &manifest,
        "doc",
        &["--locked", "--offline", "--all-features", "--no-deps"],
    )?;
    Ok(())
}

pub(super) fn phoenix(context: &DevContext) -> Result<()> {
    let web = context.repository_root.join("web");
    require_mix_project(&web)?;
    phase("Phoenix formatting, architecture, and tests (pinned Elixir container)");
    run_process(&mut web_mix_command(
        context,
        "mix format --check-formatted && mix hephaestus.architecture && mix test",
    ))
}

pub(super) fn ui(context: &DevContext) -> Result<()> {
    let web = context.repository_root.join("web");
    require_mix_project(&web)?;
    release_ui_kit(context)?;
    ui_node_tests(context)?;
    installed_ui_harness_checks(context)?;
    phase("UI architecture and focused tests (pinned Elixir container)");
    run_process(&mut web_mix_command(context, UI_CHECKS))
}

fn ui_node_tests(context: &DevContext) -> Result<()> {
    phase("Installed UI navigation and bootstrap Node checks");
    run_process(
        Command::new("node")
            .args([
                "--test",
                "--test-isolation=none",
                "--test-reporter=tap",
                "web/assets/js/design_system/hooks/installed_ui_navigation_test.mjs",
                "crates/hephaestus-app/tests/ui_bootstrap_script.mjs",
                "e2e/playwright/reporter-tests/safe-installed-ui-reporter.test.mjs",
            ])
            .current_dir(&context.repository_root),
    )
}

fn installed_ui_harness_checks(context: &DevContext) -> Result<()> {
    phase("Installed UI bridge protocol checks");
    run_process(
        Command::new(
            context
                .repository_root
                .join("scripts/test-ui-e2e-installed-bridge.sh"),
        )
        .current_dir(&context.repository_root),
    )
}

fn release_ui_kit(context: &DevContext) -> Result<()> {
    let kit = context.repository_root.join(RELEASE_UI_KIT_DIRECTORY);
    if !kit.join("package.json").is_file() {
        return Err(DevError::Invalid(format!(
            "release UI kit package is missing at {}",
            kit.display()
        )));
    }
    phase("Release UI kit Node checks");
    run_process(Command::new("npm").arg("test").current_dir(kit))
}

pub(super) fn full(context: &DevContext) -> Result<()> {
    println!("repository quality gate: generated code, architecture, Rust, Phoenix, UI");
    println!("quality phases: {}", QUALITY_FAMILIES.join(", "));
    println!("migration-gated check families are reported explicitly by their commands");
    architecture::run(context)?;
    protobuf::run(context)?;
    rust::run(context)?;
    phoenix(context)?;
    ui(context)
}

#[cfg(test)]
mod tests {
    use super::{UI_CHECKS, web_mix_command};
    use crate::context::{DevContext, ELIXIR_IMAGE};
    use std::{ffi::OsStr, path::PathBuf};

    #[test]
    fn web_checks_use_the_pinned_container_without_host_mix() {
        let context = DevContext {
            repository_root: PathBuf::from("/work/hephaestus"),
            local_root: PathBuf::from("/work/hephaestus/.local"),
            runtime_root: PathBuf::from("/tmp/hephaestus-runtime-test"),
            secret_runtime_root: PathBuf::from("/dev/shm/hephaestus-runtime-test"),
            namespace: String::from("hephaestus-local"),
            postgres_port: 55432,
            zot_port: 55000,
        };
        let command = web_mix_command(&context, "mix hephaestus.architecture --family ui");
        assert_eq!(command.get_program(), OsStr::new("podman"));
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--volume", "/work/hephaestus:/workspace:z"])
        );
        assert!(
            arguments
                .windows(2)
                .any(|pair| pair == ["--env", "MIX_ENV=test"])
        );
        assert!(arguments.iter().any(|argument| argument == ELIXIR_IMAGE));
        assert!(arguments.last().is_some_and(|script| {
            script.contains("apt-get install -y -qq --no-install-recommends git ca-certificates")
                && script.contains("mix local.hex --force")
                && script.contains("mix deps.get")
                && script.contains("mix hephaestus.architecture --family ui")
        }));
    }

    #[test]
    fn ui_checks_include_the_complete_constraint_one_scope() {
        let context = DevContext {
            repository_root: PathBuf::from("/work/hephaestus"),
            local_root: PathBuf::from("/work/hephaestus/.local"),
            runtime_root: PathBuf::from("/tmp/hephaestus-runtime-test"),
            secret_runtime_root: PathBuf::from("/dev/shm/hephaestus-runtime-test"),
            namespace: String::from("hephaestus-local"),
            postgres_port: 55432,
            zot_port: 55000,
        };
        let command = web_mix_command(&context, UI_CHECKS);
        let script = command
            .get_args()
            .last()
            .expect("container shell script")
            .to_string_lossy();

        for required in [
            "mix hephaestus.architecture --family ui",
            "test/mix/tasks/hephaestus_architecture_test.exs",
            "test/hephaestus_web_web/components",
            "test/hephaestus_web_web/design_system",
        ] {
            assert!(
                script.contains(required),
                "missing UI check scope: {required}"
            );
        }
    }
}
