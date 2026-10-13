//! Original errors stay retained; repeated observers preserve existing classes.
use super::OciWorkerError;
use std::sync::Arc;
#[derive(Debug)]
struct Retained(Arc<OciWorkerError>);
impl std::fmt::Display for Retained {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("retained original OCI operation error")
    }
}
impl std::error::Error for Retained {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}
pub(super) fn observed(original: Arc<OciWorkerError>) -> OciWorkerError {
    use OciWorkerError::{
        BuildFailed, Filesystem, ImageNotCached, InvalidConfiguration, InvalidDockerfile,
        InvalidOutput, IsolatedVmFailed, Process, RegistryPublication, RemoteDockerfileSource,
        Serialization, Store, UnapprovedDockerfileBase, UnsafeMaterializationPath,
        UnsafeSourcePath,
    };
    match original.as_ref() {
        Store(oci_builder_worker::OciWorkerStoreError::Conflict) => {
            Store(oci_builder_worker::OciWorkerStoreError::Conflict)
        }
        Store(_) => Store(oci_builder_worker::OciWorkerStoreError::Storage(Box::new(
            Retained(original),
        ))),
        InvalidConfiguration => InvalidConfiguration,
        UnsafeSourcePath => UnsafeSourcePath,
        UnsafeMaterializationPath => UnsafeMaterializationPath,
        ImageNotCached => ImageNotCached,
        InvalidDockerfile => InvalidDockerfile,
        UnapprovedDockerfileBase => UnapprovedDockerfileBase,
        RemoteDockerfileSource => RemoteDockerfileSource,
        InvalidOutput => InvalidOutput,
        RegistryPublication => RegistryPublication,
        BuildFailed => BuildFailed,
        Filesystem(error) => Filesystem(std::io::Error::new(
            error.kind(),
            Retained(original.clone()),
        )),
        Process(error) => Process(std::io::Error::new(
            error.kind(),
            Retained(original.clone()),
        )),
        Serialization(_) => Serialization(serde_json::Error::io(std::io::Error::other(Retained(
            original,
        )))),
        IsolatedVmFailed { phase, exit_code } => IsolatedVmFailed {
            phase,
            exit_code: *exit_code,
        },
    }
}
pub(super) const fn failed(phase: &'static str) -> OciWorkerError {
    OciWorkerError::IsolatedVmFailed {
        phase,
        exit_code: None,
    }
}
