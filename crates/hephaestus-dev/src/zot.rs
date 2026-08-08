//! Local lifecycle management for the pinned forge-owned Zot registry.

use crate::{
    context::{DevContext, LOCAL_ZOT_NOTIFICATION_SINK, ZOT_IMAGE, ZOT_STORAGE_ROOT},
    process::{DevError, Result, remove_path, run, run_quiet, run_silent},
};
use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::Command,
    thread,
    time::Duration,
};

const CONFIG_TEMPLATE: &str = "deploy/zot/zot-config.json.tera";
const CONFIG_CONTAINER_PATH: &str = "/etc/zot/config.json";
const CERTIFICATE_CONTAINER_PATH: &str = "/etc/zot/verification.crt";

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
    ensure_verification_certificate(context)?;
    ensure_notification_callback_token(context)?;
    render_configuration(context)?;
    validate_configuration(context)
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
    let binary = zot_binary()?;
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
    if let Err(error) = wait_for_challenge(context) {
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
    let status = curl_challenge(&context.zot_url(), &headers)?;
    let valid = status == "401"
        && fs::read_to_string(&headers).is_ok_and(|headers| challenge_matches(context, &headers));
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

fn ensure_verification_certificate(context: &DevContext) -> Result<()> {
    let certificate = context.zot_verification_certificate();
    let signing_key = context.zot_signing_key();
    if certificate.is_file() && signing_key.is_file() {
        return Ok(());
    }
    if certificate.exists() && !certificate.is_file() {
        return Err(DevError::Invalid(format!(
            "local Zot verification certificate is not a file: {}",
            certificate.display()
        )));
    }
    if signing_key.exists() && !signing_key.is_file() {
        return Err(DevError::Invalid(format!(
            "local registry signing key is not a file: {}",
            signing_key.display()
        )));
    }
    // A partial prior initialization cannot be trusted as a matching keypair.
    remove_path(&certificate)?;
    remove_path(&signing_key)?;
    let secret_directory = signing_key
        .parent()
        .ok_or_else(|| DevError::Invalid("local registry signing key has no parent".into()))?;
    fs::create_dir_all(secret_directory)?;
    fs::set_permissions(secret_directory, fs::Permissions::from_mode(0o700))?;
    let result = run_silent(
        Command::new("openssl").args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            signing_key
                .to_str()
                .ok_or_else(|| DevError::Invalid("non-Unicode Zot key path".into()))?,
            "-out",
            certificate
                .to_str()
                .ok_or_else(|| DevError::Invalid("non-Unicode Zot certificate path".into()))?,
            "-days",
            "7",
            "-subj",
            "/CN=hephaestus-local-zot-verifier",
        ]),
    );
    result?;
    fs::set_permissions(signing_key, fs::Permissions::from_mode(0o400))?;
    fs::set_permissions(certificate, fs::Permissions::from_mode(0o444))?;
    Ok(())
}

fn ensure_notification_callback_token(context: &DevContext) -> Result<()> {
    let path = context.zot_notification_callback_token();
    if path.is_file() {
        return Ok(());
    }
    if path.exists() {
        return Err(DevError::Invalid(format!(
            "local Zot notification callback token is not a file: {}",
            path.display()
        )));
    }
    let output = Command::new("openssl")
        .args(["rand", "-hex", "32"])
        .output()?;
    if !output.status.success()
        || output.stdout.len() != 65
        || output.stdout[..64]
            .iter()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
        || output.stdout[64] != b'\n'
    {
        return Err(DevError::Invalid(
            "OpenSSL did not produce a canonical callback token".into(),
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(&output.stdout[..64])?;
    file.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o400))?;
    Ok(())
}

fn render_configuration(context: &DevContext) -> Result<()> {
    let template = fs::read_to_string(context.repository_root.join(CONFIG_TEMPLATE))?;
    let callback_token = fs::read_to_string(context.zot_notification_callback_token())?;
    let rendered = render_template(&template, context, &callback_token)?;
    let _: serde_json::Value = serde_json::from_str(&rendered).map_err(|error| {
        DevError::Invalid(format!(
            "rendered local Zot configuration is invalid JSON: {error}"
        ))
    })?;
    let path = context.zot_config();
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    // A previously interrupted render can leave this exact disposable file;
    // never touch any other state while recovering it.
    remove_path(&temporary)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    output.write_all(rendered.as_bytes())?;
    output.sync_all()?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o444))?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn validate_configuration(context: &DevContext) -> Result<()> {
    let config = format!(
        "{}:{CONFIG_CONTAINER_PATH}:ro,Z",
        context.zot_config().display()
    );
    let certificate = format!(
        "{}:{CERTIFICATE_CONTAINER_PATH}:ro,Z",
        context.zot_verification_certificate().display()
    );
    let binary = zot_binary()?;
    run(Command::new("podman")
        .args(["run", "--rm", "--read-only"])
        .args(["--tmpfs", "/tmp:rw,noexec,nosuid,nodev"])
        .args(["--cap-drop", "all", "--security-opt", "no-new-privileges"])
        .args(["--volume", &config, "--volume", &certificate])
        .args([
            "--entrypoint",
            &binary,
            ZOT_IMAGE,
            "verify",
            CONFIG_CONTAINER_PATH,
        ]))
}

fn wait_for_challenge(context: &DevContext) -> Result<()> {
    let headers = context.zot_root().join("challenge-headers");
    let result = (|| {
        for _attempt in 0..100 {
            let status = curl_challenge(&context.zot_url(), &headers)?;
            if status == "401"
                && fs::read_to_string(&headers)
                    .is_ok_and(|value| challenge_matches(context, &value))
            {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err(DevError::Invalid(format!(
            "timed out waiting for Zot Bearer challenge at {}/v2/",
            context.zot_url()
        )))
    })();
    let _ignored = remove_path(&headers);
    result
}

fn curl_challenge(url: &str, headers: &Path) -> Result<String> {
    let result = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--output",
            "/dev/null",
            "--dump-header",
        ])
        .arg(headers)
        .args(["--write-out", "%{http_code}"])
        .arg(format!("{url}/v2/"))
        .output()?;
    if result.status.success() {
        Ok(String::from_utf8_lossy(&result.stdout).trim().into())
    } else {
        Ok(String::new())
    }
}

fn render_template(template: &str, context: &DevContext, callback_token: &str) -> Result<String> {
    let replacements = [
        ("{{ zot.storage_root }}", ZOT_STORAGE_ROOT),
        ("{{ zot.private_address }}", "0.0.0.0"),
        ("{{ zot.private_port }}", &context.zot_port.to_string()),
        (
            "{{ hephaestus.registry_token_realm }}",
            DevContext::zot_token_realm(),
        ),
        ("{{ hephaestus.registry_service }}", &context.zot_service()),
        (
            "{{ hephaestus.registry_notification_sink_url }}",
            LOCAL_ZOT_NOTIFICATION_SINK,
        ),
        (
            "{{ hephaestus.registry_notification_callback_token }}",
            callback_token,
        ),
    ];
    let mut rendered = template.to_owned();
    for (placeholder, value) in replacements {
        rendered = rendered.replace(placeholder, value);
    }
    if rendered.contains("{{") || rendered.contains("}}") {
        return Err(DevError::Invalid(
            "local Zot configuration contains an unresolved template input".into(),
        ));
    }
    Ok(rendered)
}

fn challenge_matches(context: &DevContext, headers: &str) -> bool {
    let normalized = headers.to_ascii_lowercase();
    normalized.contains("www-authenticate: bearer ")
        && normalized.contains(&format!("realm=\"{}\"", DevContext::zot_token_realm()))
        && normalized.contains(&format!("service=\"{}\"", context.zot_service()))
}

fn zot_binary() -> Result<String> {
    match env::consts::ARCH {
        "x86_64" => Ok("/usr/local/bin/zot-linux-amd64".into()),
        "aarch64" => Ok("/usr/local/bin/zot-linux-arm64".into()),
        architecture => Err(DevError::Invalid(format!(
            "the pinned Zot image does not support local host architecture {architecture}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{challenge_matches, render_template, start, stop};
    use crate::context::DevContext;
    use std::{env, path::PathBuf};
    use tempfile::{tempdir, tempdir_in};

    fn context() -> DevContext {
        DevContext {
            repository_root: PathBuf::from("/work/hephaestus"),
            local_root: PathBuf::from("/work/hephaestus/.local"),
            runtime_root: PathBuf::from("/tmp/hephaestus-runtime-test"),
            secret_runtime_root: PathBuf::from("/dev/shm/hephaestus-runtime-test"),
            namespace: String::from("hephaestus-local"),
            postgres_port: 55432,
            zot_port: 55000,
        }
    }

    #[test]
    fn rendered_configuration_has_no_template_values() {
        let rendered = render_template(
            r#"{"storage":"{{ zot.storage_root }}","port":{{ zot.private_port }},"realm":"{{ hephaestus.registry_token_realm }}","service":"{{ hephaestus.registry_service }}","address":"{{ zot.private_address }}"}"#,
            &context(),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("render configuration");
        assert!(rendered.contains("/var/lib/registry"));
        assert!(rendered.contains("55000"));
        assert!(!rendered.contains("{{"));
    }

    #[test]
    fn readiness_requires_the_expected_bearer_challenge() {
        let context = context();
        assert!(challenge_matches(
            &context,
            "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Bearer realm=\"http://127.0.0.1:8080/v1/registry/token\",service=\"localhost:55000\"\r\n"
        ));
        assert!(!challenge_matches(
            &context,
            "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"local\"\r\n"
        ));
    }

    #[test]
    #[ignore = "requires Podman, the pinned Zot image, and a loopback port"]
    fn pinned_zot_starts_with_an_authenticated_challenge() {
        let local = tempdir().expect("local state");
        let secret = tempdir_in("/dev/shm").expect("secret runtime");
        let crate_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repository_root = crate_directory
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root")
            .canonicalize()
            .expect("canonical workspace root");
        let context = DevContext {
            repository_root,
            local_root: local.path().to_owned(),
            runtime_root: PathBuf::from("/tmp/hephaestus-runtime-test"),
            secret_runtime_root: secret.path().to_owned(),
            namespace: format!("hephaestus-zot-test-{}", std::process::id()),
            postgres_port: 55432,
            zot_port: 55001,
        };
        start(&context).expect("start pinned Zot");
        stop(&context).expect("stop pinned Zot");
    }
}
