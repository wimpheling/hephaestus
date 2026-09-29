use oci_builder_runtime_local::LocalOciRuntimeConfig;
use secret_store::LocalKeyProvider;
use std::{
    error::Error,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub fn append_repository_image_mount_roots(
    mount_roots: &mut Vec<PathBuf>,
    runtime: &LocalOciRuntimeConfig,
    verification_root: &Path,
) {
    // Repository-image operational VMs mount only these private,
    // administrator-configured directories. The operation spec still selects
    // one exact job-derived child from each root.
    mount_roots.push(runtime.checkout_root.clone());
    mount_roots.push(runtime.output_root.clone());
    mount_roots.push(verification_root.to_path_buf());
    mount_roots.extend(runtime.image_layouts.values().cloned());
}

pub fn load_secret_keys(
    directory: &Path,
    active_reference: String,
) -> Result<LocalKeyProvider, Box<dyn Error>> {
    if !directory.is_absolute() {
        return Err("HEPHAESTUS_SECRET_KEY_DIRECTORY must be absolute".into());
    }
    let metadata = std::fs::symlink_metadata(directory)?;
    let process_uid = std::fs::metadata("/proc/self")?.uid();
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != process_uid
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err("secret key directory must be service-owned mode 0700".into());
    }
    let mut paths = std::fs::read_dir(directory)?
        .map(|entry| entry.map(|value| value.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut keys = Vec::with_capacity(paths.len());
    for key_path in paths {
        let key_metadata = std::fs::symlink_metadata(&key_path)?;
        if key_metadata.file_type().is_symlink()
            || !key_metadata.is_file()
            || key_metadata.uid() != process_uid
            || key_metadata.permissions().mode() & 0o777 != 0o400
        {
            return Err("secret key files must be service-owned regular mode 0400".into());
        }
        let reference = key_path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("secret key reference filename must be UTF-8")?
            .to_owned();
        let bytes = Zeroizing::new(std::fs::read(&key_path)?);
        if bytes.len() != 32 {
            return Err("secret key files must contain exactly 32 raw bytes".into());
        }
        keys.push((reference, bytes));
    }
    LocalKeyProvider::new(active_reference, keys).map_err(Into::into)
}
