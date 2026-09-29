use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::Path,
};

use super::errors::io_error;
use super::{ArtifactStoreError, MAX_ARTIFACT_BYTES};

pub fn copy_and_hash(
    source: &Path,
    destination: &Path,
) -> Result<([u8; 32], u64), ArtifactStoreError> {
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(o_nofollow())
        .open(source)
        .map_err(io_error)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .custom_flags(o_nofollow())
        .open(destination)
        .map_err(io_error)?;
    let mut digest = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(io_error)?;
        if count == 0 {
            break;
        }
        length = length
            .checked_add(u64::try_from(count).map_err(|_| ArtifactStoreError::TotalSize)?)
            .ok_or(ArtifactStoreError::TotalSize)?;
        if length > MAX_ARTIFACT_BYTES {
            return Err(ArtifactStoreError::TotalSize);
        }
        digest.update(&buffer[..count]);
        output.write_all(&buffer[..count]).map_err(io_error)?;
    }
    output.flush().map_err(io_error)?;
    Ok((digest.finalize().into(), length))
}

pub fn hash_file(source: &Path) -> Result<([u8; 32], u64), ArtifactStoreError> {
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(o_nofollow())
        .open(source)
        .map_err(io_error)?;
    let mut digest = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(io_error)?;
        if count == 0 {
            break;
        }
        length = length
            .checked_add(u64::try_from(count).map_err(|_| ArtifactStoreError::TotalSize)?)
            .ok_or(ArtifactStoreError::TotalSize)?;
        if length > MAX_ARTIFACT_BYTES {
            return Err(ArtifactStoreError::TotalSize);
        }
        digest.update(&buffer[..count]);
    }
    Ok((digest.finalize().into(), length))
}

#[cfg(target_os = "linux")]
pub const fn o_nofollow() -> i32 {
    0o400_000 | 0o2_000_000
}

#[cfg(not(target_os = "linux"))]
pub const fn o_nofollow() -> i32 {
    0
}
