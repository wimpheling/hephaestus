use crate::{
    context::DevContext,
    process::{DevError, Result, remove_path},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

pub(super) const TOOL_IMAGE: &str = "localhost/hephaestus-platform-build-tools:dev";
pub(super) const UBUNTU_BASE: &str = "docker.io/library/ubuntu@sha256:4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90";
pub(super) const UBUNTU_BASE_REPOSITORY: &str = "platform/bases/ubuntu";
pub(super) const UBUNTU_BASE_TAG: &str =
    "heph-sha256-4fbb8e6a8395de5a7550b33509421a2bafbc0aab6c06ba2cef9ebffbc7092d90";
pub(super) const IMPORT_RECEIPT_VERSION: u8 = 1;

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct ImportedBaseReceipt {
    pub(super) version: u8,
    pub(super) upstream_reference: String,
    pub(super) upstream_manifest_digest: String,
    pub(super) local_reference: String,
    pub(super) local_manifest_digest: String,
}

pub(super) fn print_base_import(context: &DevContext) -> Result<()> {
    let path = base_import_receipt_path(context);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            let receipt = read_base_receipt(&path)?;
            println!(
                "platform base import      {:>10}  {} ({})",
                metadata.len(),
                receipt.local_reference,
                receipt.upstream_manifest_digest
            );
            Ok(())
        }
        Ok(_) => Err(DevError::Invalid(format!(
            "platform base import receipt must be a non-symlink file: {}",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "platform base import      {:>10}  missing ({})",
                0,
                path.display()
            );
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn base_import_receipt_path(context: &DevContext) -> PathBuf {
    context.platform_image_base_imports().join("ubuntu.json")
}

pub(super) fn expected_base_receipt(context: &DevContext) -> ImportedBaseReceipt {
    let digest = ubuntu_base_digest().to_owned();
    ImportedBaseReceipt {
        version: IMPORT_RECEIPT_VERSION,
        upstream_reference: UBUNTU_BASE.into(),
        upstream_manifest_digest: digest.clone(),
        local_reference: format!(
            "{}/{UBUNTU_BASE_REPOSITORY}:{UBUNTU_BASE_TAG}",
            context.zot_service()
        ),
        local_manifest_digest: digest,
    }
}

pub(super) fn ubuntu_base_digest() -> &'static str {
    UBUNTU_BASE.split_once('@').map_or("", |(_, digest)| digest)
}

pub(super) fn read_base_receipt(path: &Path) -> Result<ImportedBaseReceipt> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(DevError::Invalid(format!(
            "platform base import receipt must be a non-symlink file: {}",
            path.display()
        )));
    }
    let receipt = serde_json::from_slice(&fs::read(path)?).map_err(|error| {
        DevError::Invalid(format!(
            "platform base import receipt is not valid JSON: {} ({error})",
            path.display()
        ))
    })?;
    validate_base_receipt(&receipt)?;
    Ok(receipt)
}

pub(super) fn validate_base_receipt(receipt: &ImportedBaseReceipt) -> Result<()> {
    if receipt.version != IMPORT_RECEIPT_VERSION
        || receipt.upstream_reference != UBUNTU_BASE
        || receipt.upstream_manifest_digest != ubuntu_base_digest()
        || receipt.local_manifest_digest != ubuntu_base_digest()
    {
        return Err(DevError::Invalid(
            "platform base import receipt does not describe the reviewed Ubuntu digest".into(),
        ));
    }
    Ok(())
}

pub(super) fn write_base_receipt(path: &Path, receipt: &ImportedBaseReceipt) -> Result<()> {
    let contents = serde_json::to_vec_pretty(receipt).map_err(|error| {
        DevError::Invalid(format!(
            "could not serialize platform base import receipt: {error}"
        ))
    })?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let _ignored = remove_path(&temporary);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&contents)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    fs::rename(temporary, path)?;
    Ok(())
}
