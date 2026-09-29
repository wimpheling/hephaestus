//! OCI image cache initialization and guest image preparation.

use crate::{
    build,
    cli::BuildSelection,
    context::{DEFAULT_LOCAL_OCI_IMAGE, DevContext},
    process::{DevError, Result, path_argument, remove_path, run, run_silent},
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Materializes each configured immutable OCI image into the daemon-owned
/// cache. Images are deliberately not classified by execution phase: the
/// resulting digest-keyed cache is shared by build and guest consumers.
pub(super) fn initialize_image_cache(context: &DevContext) -> Result<()> {
    build::build(context, &BuildSelection::runtime_only())?;
    let cache = context.image_cache();
    fs::create_dir_all(&cache)?;
    if fs::symlink_metadata(&cache)?.file_type().is_symlink() {
        return Err(DevError::Invalid(
            "OCI image cache must not be a symbolic link".into(),
        ));
    }
    let cache = fs::canonicalize(cache)?;
    let references = configured_oci_images()?;
    for reference in &references {
        let digest = image_digest(reference)?;
        let destination = cache.join(format!("sha256-{digest}"));
        if destination.join(".hephaestus-image").is_file() {
            let metadata = fs::symlink_metadata(&destination)?;
            let marker = destination.join(".hephaestus-image");
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || fs::symlink_metadata(&marker)?.file_type().is_symlink()
            {
                return Err(DevError::Invalid(format!(
                    "OCI image cache entry is unsafe: {}",
                    destination.display()
                )));
            }
            let recorded = fs::read_to_string(marker)?;
            if recorded.trim() != reference {
                return Err(DevError::Invalid(format!(
                    "OCI image cache entry has an unexpected immutable reference: {}",
                    destination.display()
                )));
            }
        } else {
            import_oci_image(context, reference, digest, &cache, &destination)?;
        }
    }
    build::install_guest_bootstrap(context)
        .and_then(|()| write_image_manifest(context, &references))
}

fn import_oci_image(
    context: &DevContext,
    reference: &str,
    digest: &str,
    cache: &Path,
    destination: &Path,
) -> Result<()> {
    let staging = cache.join(format!(".sha256-{digest}.{}", std::process::id()));
    remove_path(&staging)?;
    fs::create_dir(&staging)?;
    let container = context.image_container(digest);
    let _ignored = run_silent(Command::new("podman").args(["rm", "--force", &container]));
    let import_result = run(Command::new("podman").args(["pull", reference]))
        .and_then(|()| {
            run_silent(Command::new("podman").args([
                "create",
                "--name",
                &container,
                reference,
                "/bin/true",
            ]))
        })
        .and_then(|()| export_container(&container, &staging))
        .and_then(|()| configure_guest_identity(&staging))
        .and_then(|()| {
            fs::write(staging.join(".hephaestus-image"), reference)?;
            fs::rename(&staging, destination)?;
            Ok(())
        });
    let _ignored = run_silent(Command::new("podman").args(["rm", "--force", &container]));
    if import_result.is_err() {
        let _ignored = remove_path(&staging);
    }
    import_result
}

pub(super) fn configured_oci_images() -> Result<Vec<String>> {
    let configured = env::var("HEPHAESTUS_LOCAL_OCI_IMAGES")
        .unwrap_or_else(|_| String::from(DEFAULT_LOCAL_OCI_IMAGE));
    let mut references = configured
        .split(',')
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if references.is_empty() || references.iter().any(String::is_empty) {
        return Err(DevError::Invalid(
            "HEPHAESTUS_LOCAL_OCI_IMAGES must be a non-empty comma-separated list".into(),
        ));
    }
    references.sort_unstable();
    references.dedup();
    for reference in &references {
        let _ignored = image_digest(reference)?;
    }
    Ok(references)
}

pub(super) fn image_digest(reference: &str) -> Result<&str> {
    let Some((name, digest)) = reference.rsplit_once("@sha256:") else {
        return Err(DevError::Invalid(format!(
            "OCI image reference must be pinned with @sha256: {reference:?}"
        )));
    };
    let name_is_safe = !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'/' | b':' | b'_' | b'-')
        });
    if !name_is_safe
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DevError::Invalid(format!(
            "OCI image reference is not a safe immutable SHA-256 reference: {reference:?}"
        )));
    }
    Ok(digest)
}

#[derive(Serialize)]
struct ImageManifest {
    version: u32,
    roots: BTreeMap<String, ImageManifestEntry>,
}

#[derive(Serialize)]
struct ImageManifestEntry {
    kind: &'static str,
    path: PathBuf,
}

fn write_image_manifest(context: &DevContext, references: &[String]) -> Result<()> {
    let cache = fs::canonicalize(context.image_cache())?;
    let mut roots = BTreeMap::new();
    for reference in references {
        let digest = image_digest(reference)?;
        let path = fs::canonicalize(cache.join(format!("sha256-{digest}")))?;
        let marker = path.join(".hephaestus-image");
        if !path.starts_with(&cache)
            || fs::symlink_metadata(&path)?.file_type().is_symlink()
            || fs::symlink_metadata(&marker)?.file_type().is_symlink()
            || !marker.is_file()
        {
            return Err(DevError::Invalid(format!(
                "OCI image cache entry is unsafe: {}",
                path.display()
            )));
        }
        roots.insert(
            reference.clone(),
            ImageManifestEntry {
                kind: "directory",
                path,
            },
        );
    }
    let manifest = context.image_manifest();
    let temporary = manifest.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(
        &temporary,
        serde_json::to_vec(&ImageManifest { version: 1, roots }).map_err(|error| {
            DevError::Invalid(format!("cannot encode OCI image manifest: {error}"))
        })?,
    )?;
    fs::rename(temporary, manifest)?;
    Ok(())
}

fn export_container(container: &str, destination: &Path) -> Result<()> {
    let mut exporter = Command::new("podman")
        .args(["export", container])
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = exporter
        .stdout
        .take()
        .ok_or_else(|| DevError::Invalid("podman export did not expose stdout".into()))?;
    let tar_status = Command::new("tar")
        .arg("-C")
        .arg(path_argument(destination))
        .args(["-xf", "-"])
        .stdin(stdout)
        .status()?;
    let export_status = exporter.wait()?;
    if !tar_status.success() {
        return Err(DevError::Command {
            program: "tar".into(),
            status: tar_status,
        });
    }
    if !export_status.success() {
        return Err(DevError::Command {
            program: "podman".into(),
            status: export_status,
        });
    }
    Ok(())
}

fn configure_guest_identity(root: &Path) -> Result<()> {
    let passwd = root.join("etc/passwd");
    let group = root.join("etc/group");
    if contains_numeric_identity(&passwd, 2, "10001")?
        || contains_numeric_identity(&group, 2, "10001")?
    {
        return Err(DevError::Invalid(
            "the pinned root image already assigns guest UID/GID 10001".into(),
        ));
    }
    OpenOptions::new()
        .append(true)
        .open(passwd)?
        .write_all(b"heph-agent:x:10001:10001:Hephaestus agent:/nonexistent:/sbin/nologin\n")?;
    OpenOptions::new()
        .append(true)
        .open(group)?
        .write_all(b"heph-agent:x:10001:\n")?;
    Ok(())
}

pub(super) fn contains_numeric_identity(path: &Path, field: usize, expected: &str) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    Ok(content
        .lines()
        .any(|line| line.split(':').nth(field) == Some(expected)))
}
