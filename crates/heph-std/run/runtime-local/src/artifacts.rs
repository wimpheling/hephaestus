use release_domain::ArtifactPath;
use run_orchestrator::{RunRuntimeArtifact, RunRuntimeArtifactKind, RunRuntimeError};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

use crate::filesystem::{filesystem, o_nofollow, runtime_error, serialization};

pub fn materialize_artifact(
    store_root: &Path,
    release_root: &Path,
    artifact: &RunRuntimeArtifact,
) -> Result<(), RunRuntimeError> {
    let relative = ArtifactPath::parse(artifact.path.clone())
        .map_err(|_| runtime_error("release artifact path is invalid"))?;
    let expected_mode = match artifact.kind {
        RunRuntimeArtifactKind::Executable => 0o555,
        RunRuntimeArtifactKind::File | RunRuntimeArtifactKind::Manifest => 0o444,
    };
    if artifact.mode != expected_mode {
        return Err(runtime_error("release artifact metadata is invalid"));
    }
    let source = store_root.join(artifact.storage_key.simple().to_string());
    let destination = release_root.join(relative.as_str());
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(filesystem)?;
    }
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(o_nofollow())
        .open(source)
        .map_err(filesystem)?;
    let metadata = input.metadata().map_err(filesystem)?;
    if !metadata.file_type().is_file()
        || metadata.nlink() != 1
        || metadata.len() != artifact.size_bytes
    {
        return Err(runtime_error("canonical release object is invalid"));
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .custom_flags(o_nofollow())
        .open(&destination)
        .map_err(filesystem)?;
    let mut digest = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(filesystem)?;
        if count == 0 {
            break;
        }
        length = length
            .checked_add(
                u64::try_from(count)
                    .map_err(|_| runtime_error("release artifact size is invalid"))?,
            )
            .ok_or_else(|| runtime_error("release artifact size is invalid"))?;
        digest.update(&buffer[..count]);
        output.write_all(&buffer[..count]).map_err(filesystem)?;
    }
    output.flush().map_err(filesystem)?;
    if length != artifact.size_bytes || <[u8; 32]>::from(digest.finalize()) != artifact.content_hash
    {
        return Err(runtime_error(
            "canonical release object failed verification",
        ));
    }
    fs::set_permissions(destination, fs::Permissions::from_mode(expected_mode)).map_err(filesystem)
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), RunRuntimeError> {
    let bytes = serde_json::to_vec(value).map_err(serialization)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .custom_flags(o_nofollow())
        .open(path)
        .map_err(filesystem)?;
    file.write_all(&bytes).map_err(filesystem)?;
    file.write_all(b"\n").map_err(filesystem)?;
    file.flush().map_err(filesystem)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o444)).map_err(filesystem)
}

pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), RunRuntimeError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .custom_flags(o_nofollow())
        .open(path)
        .map_err(filesystem)?;
    file.write_all(bytes).map_err(filesystem)?;
    file.flush().map_err(filesystem)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o444)).map_err(filesystem)
}
