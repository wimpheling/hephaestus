use super::{CERTIFICATE_CONTAINER_PATH, CONFIG_CONTAINER_PATH, configuration};
use crate::{
    context::{DevContext, ZOT_IMAGE, ZOT_STORAGE_ROOT},
    process::{Result, remove_path, run, run_quiet, run_silent},
};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

/// Create non-secret local Zot state and validate the rendered configuration
/// with the same immutable Zot artifact that will run it.
pub fn initialize(context: &DevContext) -> Result<()> {
    fs::create_dir_all(context.zot_root())?;
    fs::set_permissions(context.zot_root(), fs::Permissions::from_mode(0o700))?;
    fs::create_dir_all(context.zot_storage())?;
    // Zot's image storage is non-secret content. The capability-free Zot
    // process can run under a remapped UID in rootless Podman, so the bind
    // mount itself must be writable independently of that mapping. The
    // enclosing local-state directory remains private and all credentials
    // remain in the separate 0700 secrets directory.
    fs::set_permissions(context.zot_storage(), fs::Permissions::from_mode(0o777))?;
    configuration::ensure_verification_certificate(context)?;
    configuration::ensure_notification_callback_token(context)?;
    configuration::render_configuration(context)?;
    configuration::validate_configuration(context)
}

/// Start Zot before the main local stack. It is intentionally independent of
/// the shell runner so that it has a dedicated lifecycle, state resource, and
/// diagnostics surface.
pub fn start(context: &DevContext) -> Result<()> {
    initialize(context)?;
    let _ignored = stop(context);
    let container = context.zot_container();
    let publish = format!("127.0.0.1:{}:{}", context.zot_port, context.zot_port);
    let config = format!(
        "{}:{CONFIG_CONTAINER_PATH}:ro,Z",
        context.zot_config().display()
    );
    let certificate = format!(
        "{}:{CERTIFICATE_CONTAINER_PATH}:ro,Z",
        context.zot_verification_certificate().display()
    );
    let storage = format!(
        // Persistent storage must survive container replacement. Use Podman's
        // shared SELinux label so each fresh local Zot container can access
        // the same rootless bind mount without an MCS-label mismatch.
        "{}:{ZOT_STORAGE_ROOT}:rw,z",
        context.zot_storage().display()
    );
    let binary = configuration::zot_binary()?;
    let start_result = run_silent(
        Command::new("podman")
            .args(["run", "--detach", "--rm", "--name", &container])
            .args(["--publish", &publish])
            .args(["--read-only", "--tmpfs", "/tmp:rw,noexec,nosuid,nodev"])
            .args(["--cap-drop", "all"])
            .args(["--security-opt", "no-new-privileges"])
            // Rootless Podman cannot reliably retain an MCS label across this
            // persistent bind mount. The mount is already inside private
            // local state, and this container receives only the three
            // explicitly declared mounts, so disable label separation for
            // this local-only service rather than making Zot storage flaky.
            .args(["--security-opt", "label=disable"])
            // Keep the image's default service user. `keep-id` would make the
            // process use the developer UID, which does not match Zot's
            // storage access model under a rootless user namespace. Container
            // root is still mapped to the invoking developer on the host.
            .args(["--volume", &config])
            .args(["--volume", &certificate])
            .args(["--volume", &storage])
            .args([
                "--entrypoint",
                &binary,
                ZOT_IMAGE,
                "serve",
                CONFIG_CONTAINER_PATH,
            ]),
    );
    if let Err(error) = start_result {
        let _ignored = stop(context);
        return Err(error);
    }
    if let Err(error) = configuration::wait_for_challenge(context) {
        let _ignored = stop(context);
        return Err(error);
    }
    println!(
        "Zot registry ready at {} (Bearer challenge verified)",
        context.zot_url()
    );
    Ok(())
}

/// Stop only the container named by this local namespace; registry storage is
/// deliberately retained until the `state clean --zot` operation.
pub fn stop(context: &DevContext) -> Result<()> {
    run_silent(Command::new("podman").args(["rm", "--force", &context.zot_container()]))
}

pub fn clean(context: &DevContext) -> Result<()> {
    let _ignored = stop(context);
    remove_path(&context.zot_root())
}

pub fn running(context: &DevContext) -> Result<bool> {
    run_quiet(
        "podman",
        &[
            "inspect",
            "--format",
            "{{.State.Running}}",
            &context.zot_container(),
        ],
    )
}

pub fn challenge_ready(context: &DevContext) -> Result<bool> {
    let headers = context.zot_root().join("challenge-headers");
    let status = configuration::curl_challenge(&context.zot_url(), &headers)?;
    let valid = status == "401"
        && fs::read_to_string(&headers)
            .is_ok_and(|headers| configuration::challenge_matches(context, &headers));
    let _ignored = remove_path(&headers);
    Ok(valid)
}

pub fn show_logs(context: &DevContext, follow: bool) -> Result<()> {
    if !running(context)? {
        println!("stopped ({})", context.zot_container());
        return Ok(());
    }
    let mut command = Command::new("podman");
    command.arg("logs");
    if follow {
        command.args(["--follow", "--tail", "200"]);
    } else {
        command.args(["--tail", "200"]);
    }
    command.arg(context.zot_container());
    run(&mut command)
}
