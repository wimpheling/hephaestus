use crate::{
    cli::CheckCommand,
    context::{DevContext, ELIXIR_IMAGE},
    process::{DevError, Result, run as run_process},
};
use std::{env, path::Path, process::Command};

mod architecture;

const UI_CHECKS: &str = "mix hephaestus.architecture --family ui && mix test test/mix/tasks/hephaestus_architecture_test.exs test/hephaestus_web_web/components test/hephaestus_web_web/design_system";
const QUALITY_FAMILIES: [&str; 5] = ["protobuf", "architecture", "rust", "phoenix", "ui"];
const COOKING_SERVICE_MANIFEST: &str = "examples/cooking/cooking-service/Cargo.toml";
const RELEASE_UI_KIT_DIRECTORY: &str = "web/assets/release_ui_kit";

pub fn run(context: &DevContext, command: CheckCommand) -> Result<()> {
    match command {
        CheckCommand::Architecture => architecture::run(context),
        CheckCommand::Protobuf => protobuf(context),
        CheckCommand::Rust => rust(context),
        CheckCommand::Phoenix => phoenix(context),
        CheckCommand::Ui => ui(context),
        CheckCommand::Full => full(context),
    }
}

/// Run the complete repository quality gate through one stable command.
pub fn quality(context: &DevContext) -> Result<()> {
    full(context)
}

fn protobuf(context: &DevContext) -> Result<()> {
    let root = &context.repository_root;
    if !root.join("buf.yaml").is_file() {
        println!(
            "SKIP protobuf checks: RPC migration is gated; buf.yaml has not been introduced (Constraint 3)"
        );
        return Ok(());
    }

    let generation_script = root.join("scripts/check-generated.sh");
    if !generation_script.is_file() {
        return Err(DevError::Invalid(
            "scripts/check-generated.sh is required by RPC-GENERATED-FILES-CLEAN".to_owned(),
        ));
    }

    phase("deterministic generated bindings");
    run_process(Command::new(&generation_script).current_dir(root))?;

    let buf = root.join(".local/protobuf/bin/buf");
    phase("protobuf format");
    run_process(
        Command::new(&buf)
            .args(["format", "--diff", "--exit-code"])
            .current_dir(root),
    )?;
    phase("protobuf lint");
    run_process(Command::new(&buf).arg("lint").current_dir(root))?;

    phase("protobuf descriptor policy");
    cargo(
        root,
        &[
            "test",
            "-p",
            "rpc-proto",
            "--test",
            "descriptor_policy",
            "--all-features",
        ],
    )?;

    phase("protobuf compatibility");
    run_process(Command::new(root.join("scripts/check-protobuf-breaking.sh")).current_dir(root))?;
    Ok(())
}

fn rust(context: &DevContext) -> Result<()> {
    let root = &context.repository_root;
    let browser_session_rpc = env::var("REAL_APP_BROWSER_SESSION_RPC").as_deref() == Ok("1");
    phase("Rust formatting");
    cargo(root, &["fmt", "--all", "--", "--check"])?;
    phase("Rust Clippy");
    cargo(
        root,
        &["clippy", "--workspace", "--all-targets", "--all-features"],
    )?;
    phase("Rust tests");
    workspace_tests(root, browser_session_rpc)?;
    browser_session_lifecycle(context, browser_session_rpc)?;
    phase("Rust documentation");
    cargo(root, &["doc", "--workspace", "--all-features", "--no-deps"])?;
    cooking_service(context)
}

fn workspace_tests(root: &Path, browser_session_rpc: bool) -> Result<()> {
    let mut command = Command::new("cargo");
    command
        .args(["test", "--workspace", "--all-features"])
        .current_dir(root);
    if browser_session_rpc {
        // Run the opt-in proof after this shared pass against its disposable
        // database instead of the caller's potentially contaminated database.
        command.env_remove("REAL_APP_BROWSER_SESSION_RPC");
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

fn cooking_service(context: &DevContext) -> Result<()> {
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

fn phoenix(context: &DevContext) -> Result<()> {
    let web = context.repository_root.join("web");
    require_mix_project(&web)?;
    phase("Phoenix formatting, architecture, and tests (pinned Elixir container)");
    run_process(&mut web_mix_command(
        context,
        "mix format --check-formatted && mix hephaestus.architecture && mix test",
    ))
}

fn ui(context: &DevContext) -> Result<()> {
    let web = context.repository_root.join("web");
    require_mix_project(&web)?;
    release_ui_kit(context)?;
    phase("UI architecture and focused tests (pinned Elixir container)");
    run_process(&mut web_mix_command(context, UI_CHECKS))
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

fn full(context: &DevContext) -> Result<()> {
    println!("repository quality gate: generated code, architecture, Rust, Phoenix, UI");
    println!("quality phases: {}", QUALITY_FAMILIES.join(", "));
    println!("migration-gated check families are reported explicitly by their commands");
    architecture::run(context)?;
    protobuf(context)?;
    rust(context)?;
    phoenix(context)?;
    ui(context)
}

fn cargo(root: &Path, arguments: &[&str]) -> Result<()> {
    run_process(Command::new("cargo").args(arguments).current_dir(root))
}

fn cargo_manifest(
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

fn require_mix_project(web: &Path) -> Result<()> {
    if web.join("mix.exs").is_file() {
        Ok(())
    } else {
        Err(DevError::Invalid(format!(
            "Phoenix project is missing at {}",
            web.display()
        )))
    }
}

fn web_mix_command(context: &DevContext, checks: &str) -> Command {
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

#[cfg(test)]
mod quality_tests {
    use super::QUALITY_FAMILIES;

    #[test]
    fn quality_gate_covers_every_repository_family() {
        assert_eq!(
            QUALITY_FAMILIES,
            ["protobuf", "architecture", "rust", "phoenix", "ui"]
        );
    }
}

fn phase(name: &str) {
    println!("\n== {name} ==");
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
