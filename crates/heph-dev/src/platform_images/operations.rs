use super::{
    receipt::{
        TOOL_IMAGE, UBUNTU_BASE, UBUNTU_BASE_REPOSITORY, base_import_receipt_path,
        expected_base_receipt, print_base_import, read_base_receipt, ubuntu_base_digest,
        write_base_receipt,
    },
    registry::issue_zot_token,
    support::{
        binary_version, build_release_tools, build_tool_image, create_volume, print_directory,
    },
    validation::validate_revision,
};
use crate::{
    cli::{PlatformImageBuildArgs, PlatformImageCleanArgs, PlatformImagePublishArgs},
    context::DevContext,
    process::{DevError, Result, remove_path, run, run_quiet},
    zot,
};
use registry_token::RepositoryActions;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

/// Lists local release outputs and completed installations without starting a
/// build, scanner, registry publication, or VM materialization operation.
pub fn status(context: &DevContext) -> Result<()> {
    print_base_import(context)?;
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

/// Imports the single reviewed Ubuntu manifest from its pinned upstream
/// reference to local Zot. There is deliberately no caller-controlled source
/// argument: this command is not a general registry mirror.
pub fn import_base(context: &DevContext) -> Result<()> {
    zot::start(context)?;
    build_tool_image(context)?;
    let receipt = expected_base_receipt(context);
    let receipt_path = base_import_receipt_path(context);
    if receipt_path.exists() {
        let existing = read_base_receipt(&receipt_path)?;
        if existing == receipt {
            println!("reviewed Ubuntu base is already imported into local Zot");
            return Ok(());
        }
        return Err(DevError::Invalid(format!(
            "platform base import receipt does not match the reviewed base: {}",
            receipt_path.display()
        )));
    }
    fs::create_dir_all(context.platform_image_base_imports())?;
    fs::set_permissions(
        context.platform_image_base_imports(),
        fs::Permissions::from_mode(0o700),
    )?;
    let token = issue_zot_token(
        context,
        UBUNTU_BASE_REPOSITORY,
        RepositoryActions::pull_push(),
    )?;
    println!("importing the reviewed Ubuntu base into local Zot");
    let command = [
        "sh",
        "-ec",
        "actual=$(skopeo inspect --raw \"docker://$HEPHAESTUS_PLATFORM_UBUNTU_BASE\" | sha256sum | cut --delimiter=' ' --fields=1) && actual=sha256:$actual && test \"$actual\" = \"$HEPHAESTUS_PLATFORM_UBUNTU_DIGEST\" && skopeo copy --all --dest-tls-verify=false --dest-registry-token \"$HEPHAESTUS_PLATFORM_ZOT_TOKEN\" --preserve-digests \"docker://$HEPHAESTUS_PLATFORM_UBUNTU_BASE\" \"docker://$HEPHAESTUS_PLATFORM_UBUNTU_LOCAL_REFERENCE\" && actual=$(skopeo inspect --raw --tls-verify=false --registry-token \"$HEPHAESTUS_PLATFORM_ZOT_TOKEN\" \"docker://$HEPHAESTUS_PLATFORM_UBUNTU_LOCAL_REFERENCE\" | sha256sum | cut --delimiter=' ' --fields=1) && actual=sha256:$actual && test \"$actual\" = \"$HEPHAESTUS_PLATFORM_UBUNTU_DIGEST\"",
    ];
    let result = run(Command::new("podman")
        .args(["run", "--rm", "--network=host"])
        .args([
            "--env",
            &format!("HEPHAESTUS_PLATFORM_UBUNTU_BASE={UBUNTU_BASE}"),
        ])
        .args([
            "--env",
            &format!("HEPHAESTUS_PLATFORM_UBUNTU_DIGEST={}", ubuntu_base_digest()),
        ])
        .args([
            "--env",
            &format!(
                "HEPHAESTUS_PLATFORM_UBUNTU_LOCAL_REFERENCE={}",
                receipt.local_reference
            ),
        ])
        .args(["--env", &format!("HEPHAESTUS_PLATFORM_ZOT_TOKEN={token}")])
        .arg(TOOL_IMAGE)
        .args(command)
        .current_dir(&context.repository_root));
    if result.is_ok() {
        write_base_receipt(&receipt_path, &receipt)?;
        println!("reviewed Ubuntu base imported and verified in local Zot");
    }
    result
}

/// Runs the reviewed six-image construction script only after the caller has
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
    let base = read_base_receipt(&base_import_receipt_path(context))?;
    if base != expected_base_receipt(context) {
        return Err(DevError::Invalid(
            "local Zot base-import receipt does not match the reviewed Ubuntu base; run cargo dev platform-images import-base"
                .into(),
        ));
    }
    build_tool_image(context)?;
    create_volume(&context.platform_image_tool_storage_volume())?;
    create_volume(&context.platform_image_tool_cache_volume())?;
    println!(
        "building six platform images into {}; this explicit operation may take several minutes",
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
    let pull_token = issue_zot_token(context, UBUNTU_BASE_REPOSITORY, RepositoryActions::pull())?;
    let script_arguments = [
        "sh",
        "-ec",
        "skopeo copy --src-tls-verify=false --src-registry-token \"$HEPHAESTUS_PLATFORM_ZOT_TOKEN\" \"docker://$HEPHAESTUS_PLATFORM_UBUNTU_LOCAL_REFERENCE\" \"containers-storage:$HEPHAESTUS_PLATFORM_UBUNTU_BASE\" >/dev/null && trivy image --download-db-only >/dev/null && exec /workspace/scripts/build-platform-builder-layouts.sh --output-root \"$HEPHAESTUS_PLATFORM_RELEASE_OUTPUT_ROOT\" --source \"$HEPHAESTUS_PLATFORM_RELEASE_SOURCE\" --revision \"$HEPHAESTUS_PLATFORM_RELEASE_REVISION\" --created \"$HEPHAESTUS_PLATFORM_RELEASE_CREATED\"",
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
        .args([
            "--env",
            &format!(
                "HEPHAESTUS_PLATFORM_UBUNTU_LOCAL_REFERENCE={}",
                base.local_reference
            ),
        ])
        .args([
            "--env",
            &format!("HEPHAESTUS_PLATFORM_ZOT_TOKEN={pull_token}"),
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
        .args(["--env", "HEPHAESTUS_SYFT_VERSION=syft 1.51.1-hephaestus.1"])
        .args(["--env", "HEPHAESTUS_TRIVY=/usr/local/bin/trivy"])
        .args([
            "--env",
            "HEPHAESTUS_TRIVY_VERSION=Version: 0.74.0-hephaestus.1",
        ])
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
        "publishing six reviewed platform images and applying their local catalog; this explicit operation may take several minutes"
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
