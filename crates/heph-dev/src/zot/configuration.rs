use super::{CERTIFICATE_CONTAINER_PATH, CONFIG_CONTAINER_PATH, CONFIG_TEMPLATE};
use crate::{
    context::{DevContext, LOCAL_ZOT_NOTIFICATION_SINK, ZOT_IMAGE, ZOT_STORAGE_ROOT},
    process::{DevError, Result, remove_path, run, run_silent},
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

pub(super) fn ensure_verification_certificate(context: &DevContext) -> Result<()> {
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

pub(super) fn ensure_notification_callback_token(context: &DevContext) -> Result<()> {
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

pub(super) fn render_configuration(context: &DevContext) -> Result<()> {
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

pub(super) fn validate_configuration(context: &DevContext) -> Result<()> {
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

pub(super) fn wait_for_challenge(context: &DevContext) -> Result<()> {
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

pub(super) fn curl_challenge(url: &str, headers: &Path) -> Result<String> {
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

pub fn render_template(
    template: &str,
    context: &DevContext,
    callback_token: &str,
) -> Result<String> {
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

pub fn challenge_matches(context: &DevContext, headers: &str) -> bool {
    let normalized = headers.to_ascii_lowercase();
    normalized.contains("www-authenticate: bearer ")
        && normalized.contains(&format!("realm=\"{}\"", DevContext::zot_token_realm()))
        && normalized.contains(&format!("service=\"{}\"", context.zot_service()))
}

pub(super) fn zot_binary() -> Result<String> {
    match env::consts::ARCH {
        "x86_64" => Ok("/usr/local/bin/zot-linux-amd64".into()),
        "aarch64" => Ok("/usr/local/bin/zot-linux-arm64".into()),
        architecture => Err(DevError::Invalid(format!(
            "the pinned Zot image does not support local host architecture {architecture}"
        ))),
    }
}
