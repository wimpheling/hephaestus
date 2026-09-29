use registry_domain::{OciDescriptor, RegistryAuthority, Sha256Digest};
use reqwest::Url;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs::{self, File},
    io::{BufReader, Read},
    path::{Component, Path, PathBuf},
};

use super::{
    constants::{OCI_IMAGE_LAYOUT_VERSION, OCI_REFERENCE_NAME_ANNOTATION},
    errors::PublisherError,
    remote::{RemoteDescriptor, RemoteManifest, parse_platforms},
};

pub fn os_arguments<const N: usize>(values: [OsString; N]) -> Vec<OsString> {
    values.into()
}

pub fn registry_origin(authority: &RegistryAuthority) -> Result<Url, PublisherError> {
    parse_registry_origin(&format!("https://{authority}"))
}

pub fn parse_registry_origin(value: &str) -> Result<Url, PublisherError> {
    let origin = Url::parse(value).map_err(|_| PublisherError::UnsafeRegistryOrigin)?;
    let valid = matches!(origin.scheme(), "http" | "https")
        && origin.host_str().is_some()
        && origin.username().is_empty()
        && origin.password().is_none()
        && origin.query().is_none()
        && origin.fragment().is_none()
        && origin.path() == "/";
    valid
        .then_some(origin)
        .ok_or(PublisherError::UnsafeRegistryOrigin)
}

pub fn validate_local_layout(
    layout: &Path,
    expected: &OciDescriptor,
) -> Result<String, PublisherError> {
    let layout_file = safe_layout_file(layout, Path::new("oci-layout"))?;
    let layout_json: LocalLayoutVersion =
        serde_json::from_reader(File::open(layout_file).map_err(PublisherError::Filesystem)?)
            .map_err(|_| PublisherError::MalformedLocalLayout)?;
    if layout_json.image_layout_version != OCI_IMAGE_LAYOUT_VERSION {
        return Err(PublisherError::MalformedLocalLayout);
    }
    let index_file = safe_layout_file(layout, Path::new("index.json"))?;
    let index: LocalIndex =
        serde_json::from_reader(File::open(index_file).map_err(PublisherError::Filesystem)?)
            .map_err(|_| PublisherError::MalformedLocalIndex)?;
    let matching = index
        .manifests
        .iter()
        .filter(|descriptor| descriptor.digest == expected.digest().as_str())
        .collect::<Vec<_>>();
    let [local] = matching.as_slice() else {
        return Err(PublisherError::WrongLocalDigest);
    };
    if index.manifests.len() != 1 {
        return Err(PublisherError::AmbiguousLocalLayout);
    }
    if local.to_domain()? != *expected {
        return Err(PublisherError::WrongLocalDescriptor);
    }
    let digest_hex = expected
        .digest()
        .as_str()
        .strip_prefix("sha256:")
        .ok_or(PublisherError::WrongLocalDigest)?;
    let blob = safe_layout_file(
        layout,
        &PathBuf::from("blobs").join("sha256").join(digest_hex),
    )?;
    let metadata = fs::metadata(&blob).map_err(PublisherError::Filesystem)?;
    if metadata.len() != expected.size() || hash_file(&blob)? != *expected.digest() {
        return Err(PublisherError::WrongLocalDigest);
    }
    let subject_bytes = fs::read(&blob).map_err(PublisherError::Filesystem)?;
    let subject: RemoteManifest =
        serde_json::from_slice(&subject_bytes).map_err(|_| PublisherError::MalformedLocalIndex)?;
    parse_platforms(&subject, expected).map_err(|_| PublisherError::MalformedLocalIndex)?;
    let tag = local
        .annotations
        .get(OCI_REFERENCE_NAME_ANNOTATION)
        .filter(|value| value.as_str() == local_reference_tag(expected.digest()))
        .cloned()
        .ok_or(PublisherError::WrongLocalReferenceName)?;
    Ok(tag)
}

pub fn local_reference_tag(digest: &Sha256Digest) -> String {
    format!("heph-{}", digest.as_str().replace(':', "-"))
}

pub fn hash_file(path: &Path) -> Result<Sha256Digest, PublisherError> {
    let file = File::open(path).map_err(PublisherError::Filesystem)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16_384];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(PublisherError::Filesystem)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Sha256Digest::parse(format!("sha256:{:x}", hasher.finalize())).map_err(PublisherError::Domain)
}

pub fn canonical_directory(path: &Path) -> Result<PathBuf, PublisherError> {
    if !path.is_absolute() {
        return Err(PublisherError::UnsafePath);
    }
    let metadata = fs::symlink_metadata(path).map_err(PublisherError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(PublisherError::UnsafePath);
    }
    fs::canonicalize(path).map_err(PublisherError::Filesystem)
}

pub fn canonical_executable(path: &Path) -> Result<PathBuf, PublisherError> {
    if !path.is_absolute() {
        return Err(PublisherError::UnsafePath);
    }
    let metadata = fs::symlink_metadata(path).map_err(PublisherError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PublisherError::UnsafePath);
    }
    fs::canonicalize(path).map_err(PublisherError::Filesystem)
}

pub fn trusted_directory(root: &Path, path: &Path) -> Result<PathBuf, PublisherError> {
    let path = trusted_path(root, path)?;
    fs::metadata(&path)
        .map_err(PublisherError::Filesystem)?
        .is_dir()
        .then_some(path)
        .ok_or(PublisherError::UnsafePath)
}

pub fn trusted_file(root: &Path, path: &Path) -> Result<PathBuf, PublisherError> {
    let path = trusted_path(root, path)?;
    fs::metadata(&path)
        .map_err(PublisherError::Filesystem)?
        .is_file()
        .then_some(path)
        .ok_or(PublisherError::UnsafePath)
}

pub fn trusted_path(root: &Path, path: &Path) -> Result<PathBuf, PublisherError> {
    if !path.is_absolute() || !path.starts_with(root) {
        return Err(PublisherError::UnsafePath);
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| PublisherError::UnsafePath)?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PublisherError::UnsafePath);
    }
    let mut current = root.to_owned();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(PublisherError::UnsafePath);
        };
        current.push(component);
        if fs::symlink_metadata(&current)
            .map_err(PublisherError::Filesystem)?
            .file_type()
            .is_symlink()
        {
            return Err(PublisherError::UnsafePath);
        }
    }
    let canonical = fs::canonicalize(path).map_err(PublisherError::Filesystem)?;
    canonical
        .starts_with(root)
        .then_some(canonical)
        .ok_or(PublisherError::UnsafePath)
}

pub fn safe_layout_file(layout: &Path, relative: &Path) -> Result<PathBuf, PublisherError> {
    trusted_file(layout, &layout.join(relative))
}

pub fn is_authentication_failure(stderr: &[u8]) -> bool {
    let lower = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    ["unauthorized", "authentication", "token expired", "denied"]
        .iter()
        .any(|needle| lower.contains(needle))
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalLayoutVersion {
    image_layout_version: String,
}

#[derive(Debug, Deserialize)]
struct LocalIndex {
    manifests: Vec<RemoteDescriptor>,
}
