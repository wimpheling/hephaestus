//! Explicit local release operations for the reviewed platform builder images.

use crate::{
    cli::{PlatformImageBuildArgs, PlatformImageCleanArgs, PlatformImagePublishArgs},
    context::DevContext,
    process::{DevError, Result, directory_size, remove_path, run, run_quiet},
    zot,
};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

const TOOL_IMAGE: &str = "localhost/hephaestus-platform-build-tools:dev";
const UBUNTU_BASE: &str = "docker.io/library/ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90";

/// Lists local release outputs and completed installations without starting a
/// build, scanner, registry publication, or VM materialization operation.
pub fn status(context: &DevContext) -> Result<()> {
    print_directory(
        "platform image releases",
        &context.platform_image_releases(),
    )?;
    print_directory(
        "platform image installs",
        &context.platform_image_installations(),
    )?;
    Ok(())
}

/// Runs the reviewed four-image construction script only after the caller has
/// provided immutable release provenance. Publication remains a separate
/// explicit operation.
pub fn build(context: &DevContext, arguments: &PlatformImageBuildArgs) -> Result<()> {
    validate_revision(&arguments.revision)?;
    let release_root = context.platform_image_releases().join(&arguments.revision);
    if release_root.exists() {
        return Err(DevError::Invalid(format!(
            "platform image release output already exists: {}; use a new immutable revision or remove it explicitly",
            release_root.display()
        )));
    }
    let parent = context.platform_image_releases();
    fs::create_dir_all(&parent)?;
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700))?;
    let script = context
        .repository_root
        .join("scripts/build-platform-builder-layouts.sh");
    if !script.is_file() {
        return Err(DevError::Invalid(format!(
            "reviewed platform image build script is missing: {}",
            script.display()
        )));
    }
    build_tool_image(context)?;
    create_volume(&context.platform_image_tool_storage_volume())?;
    create_volume(&context.platform_image_tool_cache_volume())?;
    println!(
        "building four platform images into {}; this explicit operation may take several minutes",
        release_root.display()
    );
    let source_mount = format!("{}:/workspace:ro,Z", context.repository_root.display());
    // The release metadata records layout paths. Preserve the host path in the
    // tool container so publication can verify that immutable contract later.
    let output_mount = format!("{}:{}:Z", parent.display(), parent.display());
    // Named volumes are already managed by Podman; SELinux relabelling applies
    // only to host bind mounts and prevents these volume targets from resolving.
    let storage_mount = format!("{}:/state", context.platform_image_tool_storage_volume());
    let cache_mount = format!("{}:/cache", context.platform_image_tool_cache_volume());
    let source_environment = format!("HEPHAESTUS_PLATFORM_RELEASE_SOURCE={}", arguments.source);
    let revision_environment = format!(
        "HEPHAESTUS_PLATFORM_RELEASE_REVISION={}",
        arguments.revision
    );
    let created_environment = format!("HEPHAESTUS_PLATFORM_RELEASE_CREATED={}", arguments.created);
    let output_environment = format!(
        "HEPHAESTUS_PLATFORM_RELEASE_OUTPUT_ROOT={}",
        release_root.display()
    );
    let script_arguments = [
        "sh",
        "-ec",
        "buildah pull \"$HEPHAESTUS_PLATFORM_UBUNTU_BASE\" >/dev/null && trivy image --download-db-only >/dev/null && exec /workspace/scripts/build-platform-builder-layouts.sh --output-root \"$HEPHAESTUS_PLATFORM_RELEASE_OUTPUT_ROOT\" --source \"$HEPHAESTUS_PLATFORM_RELEASE_SOURCE\" --revision \"$HEPHAESTUS_PLATFORM_RELEASE_REVISION\" --created \"$HEPHAESTUS_PLATFORM_RELEASE_CREATED\"",
    ];
    // Keep the outer Podman invocation rootless, but let the tool container use
    // its mapped container root. Buildah then uses that existing user namespace
    // rather than attempting a second, unavailable rootless mapping.
    run(Command::new("podman")
        .args(["run", "--rm", "--network=host"])
        // Rootless Buildah uses fuse-overlayfs for its storage.  Passing this
        // one device avoids a privileged container while retaining isolation.
        .args(["--device", "/dev/fuse"])
        .args(["--volume", &source_mount])
        .args(["--volume", &output_mount])
        .args(["--volume", &storage_mount])
        .args(["--volume", &cache_mount])
        // These are Podman --env flags rather than Command::env calls: the
        // latter would change the host-side Podman client's HOME before it
        // can create the container.
        .args(["--env", "HOME=/state/home"])
        .args(["--env", "XDG_CACHE_HOME=/cache"])
        .args(["--env", "TRIVY_CACHE_DIR=/cache/trivy"])
        // A nested OCI runtime cannot mount proc in this rootless container;
        // Buildah's chroot isolation executes the reviewed Dockerfile safely
        // in the already-established outer user namespace instead.
        .args(["--env", "BUILDAH_ISOLATION=chroot"])
        .args([
            "--env",
            &format!("HEPHAESTUS_PLATFORM_UBUNTU_BASE={UBUNTU_BASE}"),
        ])
        .args(["--env", &source_environment])
        .args(["--env", &revision_environment])
        .args(["--env", &created_environment])
        .args(["--env", &output_environment])
        .args(["--env", "HEPHAESTUS_BUILDAH=/usr/bin/buildah"])
        .args(["--env", "HEPHAESTUS_BUILDAH_VERSION=buildah version 1.43.2"])
        .args(["--env", "HEPHAESTUS_SKOPEO=/usr/bin/skopeo"])
        .args(["--env", "HEPHAESTUS_SKOPEO_VERSION=skopeo version 1.22.2"])
        .args(["--env", "HEPHAESTUS_SYFT=/usr/local/bin/syft"])
        .args(["--env", "HEPHAESTUS_SYFT_VERSION=syft 1.50.0"])
        .args(["--env", "HEPHAESTUS_TRIVY=/usr/local/bin/trivy"])
        .args(["--env", "HEPHAESTUS_TRIVY_VERSION=Version: 0.73.0"])
        .args(["--env", "HEPHAESTUS_JQ=/usr/bin/jq"])
        .args(["--env", "HEPHAESTUS_JQ_VERSION=jq-1.8.1"])
        .arg(TOOL_IMAGE)
        .args(script_arguments)
        .current_dir(&context.repository_root))
}

/// Publishes a previously reviewed immutable release through local Zot, then
/// applies its approved catalog records.
// This deliberately keeps the complete privileged-boundary command visible in
// one place: splitting the mount and environment construction obscures review.
#[allow(clippy::too_many_lines)]
pub fn publish(context: &DevContext, arguments: &PlatformImagePublishArgs) -> Result<()> {
    validate_revision(&arguments.revision)?;
    let release = context.platform_image_releases().join(&arguments.revision);
    if !release.join(".platform-builder-release.json").is_file() {
        return Err(DevError::Invalid(format!(
            "reviewed platform image release is missing: {}",
            release.display()
        )));
    }
    let installation = context
        .platform_image_installations()
        .join(&arguments.revision);
    if installation.exists() {
        return Err(DevError::Invalid(format!(
            "platform image installation already exists for immutable revision {}; use status or clean it explicitly",
            arguments.revision
        )));
    }
    if !run_quiet(
        "podman",
        &[
            "inspect",
            "--format",
            "{{.State.Running}}",
            &context.postgres_container(),
        ],
    )? {
        return Err(DevError::Invalid(
            "local PostgreSQL is not running; start the local stack before publishing platform images".into(),
        ));
    }
    zot::start(context)?;
    build_release_tools(context)?;
    fs::create_dir_all(context.platform_image_installations())?;
    fs::set_permissions(
        context.platform_image_installations(),
        fs::Permissions::from_mode(0o700),
    )?;
    create_volume(&context.platform_image_tool_storage_volume())?;
    let source_mount = format!("{}:/workspace:ro,Z", context.repository_root.display());
    // Release inputs bind every OCI layout to its immutable absolute path.
    // Preserve that path inside the tool container so the publication contract
    // can reject layout substitution rather than weakening its validation.
    let release_mount = format!("{}:{}:ro,Z", release.display(), release.display());
    let install_mount = format!(
        "{}:/install:Z",
        context.platform_image_installations().display()
    );
    let state_mount = format!("{}:/state", context.platform_image_tool_storage_volume());
    let key_mount = format!(
        "{}:/secrets/registry-token-signing-key.pem:ro,Z",
        context.zot_signing_key().display()
    );
    let release_binary = context
        .repository_root
        .join("target/debug/hephaestus-registry-release");
    let operator_binary = context
        .repository_root
        .join("target/debug/hephaestus-operator");
    for binary in [&release_binary, &operator_binary] {
        if !binary.is_file() {
            return Err(DevError::Invalid(format!(
                "required release tool is missing: {}",
                binary.display()
            )));
        }
    }
    println!(
        "publishing four reviewed platform images and applying their local catalog; this explicit operation may take several minutes"
    );
    run(Command::new("podman")
        .args(["run", "--rm", "--network=host"])
        .args(["--volume", &source_mount])
        .args(["--volume", &release_mount])
        .args(["--volume", &install_mount])
        .args(["--volume", &state_mount])
        .args(["--volume", &key_mount])
        .args(["--volume", &format!("{}:/tools/hephaestus-registry-release:ro,Z", release_binary.display())])
        .args(["--volume", &format!("{}:/tools/hephaestus-operator:ro,Z", operator_binary.display())])
        .args(["--env", &format!("HEPHAESTUS_FORGE_REGISTRY_AUTHORITY={}", context.zot_service())])
        .args(["--env", &format!("HEPHAESTUS_REGISTRY_SERVICE={}", context.zot_service())])
        .args(["--env", &format!("HEPHAESTUS_REGISTRY_PRIVATE_ORIGIN={}/", context.zot_url())])
        .args(["--env", &format!("HEPHAESTUS_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:{}/hephaestus?sslmode=disable", context.postgres_port)])
        .args(["--env", "HEPHAESTUS_REGISTRY_TOKEN_ISSUER=http://127.0.0.1:8080/v1/registry/token"])
        .args(["--env", "HEPHAESTUS_REGISTRY_TOKEN_KEY_ID=local-v1"])
        .args(["--env", "HEPHAESTUS_REGISTRY_TOKEN_LIFETIME_SECONDS=300"])
        .args(["--env", &format!("HEPHAESTUS_PLATFORM_RELEASE_ROOT={}", release.display())])
        .args(["--env", &format!("HEPHAESTUS_REGISTRY_RELEASE_VERSION={}", binary_version(&release_binary)?)])
        .arg(TOOL_IMAGE)
        .args(["/workspace/scripts/install-platform-builder-images.sh", &arguments.revision])
        .current_dir(&context.repository_root))?;
    println!("platform images installed in local Zot and the approved builder catalog");
    Ok(())
}

/// Removes only private local installation evidence and credentials. Approved
/// Zot publication and database catalog records remain operator-owned state.
pub fn clean(context: &DevContext, arguments: &PlatformImageCleanArgs) -> Result<()> {
    validate_revision(&arguments.revision)?;
    let installation = context
        .platform_image_installations()
        .join(&arguments.revision);
    remove_path(&installation)?;
    println!(
        "removed local platform-image installation {}",
        arguments.revision
    );
    Ok(())
}

fn build_release_tools(context: &DevContext) -> Result<()> {
    run(Command::new("cargo")
        .args([
            "build",
            "-p",
            "registry-release",
            "--bin",
            "hephaestus-registry-release",
            "-p",
            "bootstrap-postgres",
            "--bin",
            "hephaestus-operator",
        ])
        .current_dir(&context.repository_root))?;
    Ok(())
}

fn binary_version(binary: &std::path::Path) -> Result<String> {
    let output = Command::new(binary).arg("--version").output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(DevError::Command {
            program: binary.display().to_string(),
            status: output.status,
        })
    }
}

fn build_tool_image(context: &DevContext) -> Result<()> {
    let tool_context = context.repository_root.join("platform/build-tools");
    println!("building pinned platform-image tool container");
    run(Command::new("podman")
        .args(["build", "--pull=never", "--tag", TOOL_IMAGE])
        .arg(tool_context)
        .current_dir(&context.repository_root))
}

fn create_volume(volume: &str) -> Result<()> {
    let exists = Command::new("podman")
        .args(["volume", "exists", volume])
        .status()?
        .success();
    if !exists {
        run(Command::new("podman").args(["volume", "create", volume]))?;
    }
    Ok(())
}

fn validate_revision(revision: &str) -> Result<()> {
    let valid_length = matches!(revision.len(), 40 | 64);
    if valid_length
        && revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(DevError::Invalid(
            "platform image revision must be a lowercase 40- or 64-character hexadecimal commit"
                .into(),
        ))
    }
}

fn print_directory(label: &str, path: &std::path::Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            println!(
                "{label:24} {:>10}  {}",
                directory_size(path),
                path.display()
            );
            Ok(())
        }
        Ok(_) => Err(DevError::Invalid(format!(
            "{label} must be a non-symlink directory: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("{label:24} {:>10}  missing ({})", 0, path.display());
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::validate_revision;

    #[test]
    fn accepts_only_immutable_lowercase_revisions() {
        assert!(validate_revision(&"a".repeat(40)).is_ok());
        assert!(validate_revision(&"b".repeat(64)).is_ok());
        assert!(validate_revision(&"A".repeat(40)).is_err());
        assert!(validate_revision("../unsafe").is_err());
    }
}
