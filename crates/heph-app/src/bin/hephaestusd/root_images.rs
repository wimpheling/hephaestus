use builder_catalog_domain::OciImageReference;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    env,
    error::Error,
    ffi::OsString,
    path::{Path, PathBuf},
};
use vm_trait::{DiskFormat, RootFilesystem};

const ROOT_IMAGE_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootImageManifest {
    version: u32,
    roots: BTreeMap<String, RootImageManifestEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RootImageManifestEntry {
    Directory {
        path: PathBuf,
    },
    Disk {
        path: PathBuf,
        format: RootImageDiskFormat,
        read_only: bool,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RootImageDiskFormat {
    Raw,
    Qcow2,
}

pub fn root_images_from_environment(
    backend_name: &str,
) -> Result<BTreeMap<String, RootFilesystem>, Box<dyn Error>> {
    let manifest = env::var_os("HEPHAESTUS_ROOT_IMAGE_MANIFEST");
    let legacy_path = env::var_os("HEPHAESTUS_ROOT_IMAGE_PATH");
    let legacy_reference = env::var_os("HEPHAESTUS_ROOT_IMAGE_REFERENCE");

    if let Some(manifest) = manifest {
        if legacy_path.is_some() || legacy_reference.is_some() {
            return Err(String::from(
            "HEPHAESTUS_ROOT_IMAGE_MANIFEST cannot be combined with the legacy root image variables",
        )
        .into());
        }
        return load_root_image_manifest(&PathBuf::from(manifest));
    }

    if backend_name == "fixture" {
        let root_image_path = legacy_path.map_or_else(
            || {
                Err(String::from(
                    "HEPHAESTUS_ROOT_IMAGE_PATH is required in fixture mode",
                ))
            },
            Ok,
        )?;
        let root_image_reference = legacy_reference.map_or_else(
            || {
                Err(String::from(
                    "HEPHAESTUS_ROOT_IMAGE_REFERENCE is required in fixture mode",
                ))
            },
            Ok,
        )?;
        return legacy_fixture_root_images(PathBuf::from(root_image_path), root_image_reference);
    }

    Err(String::from("HEPHAESTUS_ROOT_IMAGE_MANIFEST is required outside fixture mode").into())
}

pub fn legacy_fixture_root_images(
    root_image_path: PathBuf,
    root_image_reference: OsString,
) -> Result<BTreeMap<String, RootFilesystem>, Box<dyn Error>> {
    let mut roots = BTreeMap::new();
    roots.insert(
        os_string_to_string(root_image_reference, "HEPHAESTUS_ROOT_IMAGE_REFERENCE")?,
        RootImageManifestEntry::Directory {
            path: root_image_path,
        },
    );
    validate_root_image_entries(roots)
}

pub fn load_root_image_manifest(
    manifest_path: &Path,
) -> Result<BTreeMap<String, RootFilesystem>, Box<dyn Error>> {
    if !manifest_path.is_absolute() {
        return Err(String::from("root image manifest path must be absolute").into());
    }
    let manifest: RootImageManifest = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
    if manifest.version != ROOT_IMAGE_MANIFEST_VERSION {
        return Err(format!(
            "unsupported root image manifest version {}; expected {}",
            manifest.version, ROOT_IMAGE_MANIFEST_VERSION
        )
        .into());
    }
    validate_root_image_entries(manifest.roots)
}

/// Loads the worker-written project-image manifest without allowing it to add
/// arbitrary host directories or replace platform roots. Its absence is the
/// normal state before the first project image is materialized.
pub fn repository_root_images(
    manifest_path: &Path,
    rootfs_root: &Path,
) -> Result<BTreeMap<String, RootFilesystem>, Box<dyn Error>> {
    if !manifest_path.is_absolute() || !rootfs_root.is_absolute() {
        return Err(
            String::from("repository image manifest and rootfs root must be absolute").into(),
        );
    }
    if !manifest_path.exists() {
        return Ok(BTreeMap::new());
    }
    let manifest: RootImageManifest = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
    if manifest.version != ROOT_IMAGE_MANIFEST_VERSION {
        return Err(format!(
            "unsupported repository image manifest version {}; expected {}",
            manifest.version, ROOT_IMAGE_MANIFEST_VERSION
        )
        .into());
    }
    let trusted_root = std::fs::canonicalize(rootfs_root)?;
    let mut roots = BTreeMap::new();
    for (reference, entry) in manifest.roots {
        OciImageReference::parse(reference.clone()).map_err(|error| {
            format!("repository image reference {reference:?} is not digest-pinned: {error}")
        })?;
        let RootImageManifestEntry::Directory { path } = entry else {
            return Err(
                String::from("repository image roots must be materialized directories").into(),
            );
        };
        let path = materialized_path(&reference, path, true)?;
        if !path.starts_with(&trusted_root) {
            return Err(format!(
                "repository image root {reference:?} is outside the worker rootfs root"
            )
            .into());
        }
        roots.insert(reference, RootFilesystem::Directory { host_path: path });
    }
    Ok(roots)
}

fn validate_root_image_entries(
    entries: BTreeMap<String, RootImageManifestEntry>,
) -> Result<BTreeMap<String, RootFilesystem>, Box<dyn Error>> {
    if entries.is_empty() {
        return Err(String::from("root image manifest must contain at least one root").into());
    }

    entries
        .into_iter()
        .map(|(reference, entry)| {
            OciImageReference::parse(reference.clone()).map_err(|error| {
                format!("root image reference {reference:?} is not digest-pinned: {error}")
            })?;
            let root = match entry {
                RootImageManifestEntry::Directory { path } => {
                    let path = materialized_path(&reference, path, true)?;
                    RootFilesystem::Directory { host_path: path }
                }
                RootImageManifestEntry::Disk {
                    path,
                    format,
                    read_only,
                } => {
                    let path = materialized_path(&reference, path, false)?;
                    let format = match format {
                        RootImageDiskFormat::Raw => DiskFormat::Raw,
                        RootImageDiskFormat::Qcow2 => DiskFormat::Qcow2,
                    };
                    RootFilesystem::Disk {
                        host_path: path,
                        format,
                        read_only,
                    }
                }
            };
            Ok((reference, root))
        })
        .collect()
}

fn materialized_path(
    reference: &str,
    path: PathBuf,
    directory: bool,
) -> Result<PathBuf, Box<dyn Error>> {
    if !path.is_absolute() {
        return Err(
            format!("root image {reference:?} materialization path must be absolute").into(),
        );
    }
    let metadata = std::fs::symlink_metadata(&path).map_err(|error| {
        format!("root image {reference:?} materialization path cannot be inspected: {error}")
    })?;
    if metadata.file_type().is_symlink() {
        return Err(
            format!("root image {reference:?} materialization path must not be a symlink").into(),
        );
    }
    if metadata.is_dir() != directory {
        let expected = if directory { "directory" } else { "disk file" };
        return Err(
            format!("root image {reference:?} materialization path must be a {expected}").into(),
        );
    }
    Ok(std::fs::canonicalize(path)?)
}

pub fn root_filesystem_path(root: &RootFilesystem) -> PathBuf {
    match root {
        RootFilesystem::Directory { host_path } | RootFilesystem::Disk { host_path, .. } => {
            host_path.clone()
        }
        _ => unreachable!("unsupported root filesystem variant"),
    }
}

fn os_string_to_string(value: OsString, name: &str) -> Result<String, Box<dyn Error>> {
    value
        .into_string()
        .map_err(|_| format!("{name} must contain valid UTF-8").into())
}
