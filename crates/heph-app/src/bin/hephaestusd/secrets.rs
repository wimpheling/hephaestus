use oci_builder_runtime_local::LocalOciRuntimeConfig;
use std::{
    error::Error,
    path::{Path, PathBuf},
};

pub use secret_key_local::LocalKeyProvider;

pub fn load_secret_keys(
    directory: &Path,
    active_reference: String,
) -> Result<LocalKeyProvider, Box<dyn Error>> {
    LocalKeyProvider::from_directory(directory, active_reference)
}

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
