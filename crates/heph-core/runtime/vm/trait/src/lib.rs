//! Provider-neutral virtual machine abstractions for the Hephaestus runtime.

#[path = "vm_trait/errors.rs"]
mod errors;
#[path = "vm_trait/events.rs"]
mod events;
#[path = "vm_trait/guest_volume.rs"]
mod guest_volume;
#[path = "vm_trait/http.rs"]
mod http;

pub use guest_volume::{
    VmGuestVolume, deserialize_guest_volumes, validate_guest_volume_mounts,
    validate_guest_volume_set, validate_vm_guest_volumes,
};
pub use volume_domain::{
    GuestMountPath, MAX_GUEST_MOUNT_PATH_BYTES, MAX_VOLUME_SLOTS, VolumeAccessMode,
};

#[path = "vm_trait/spec.rs"]
mod spec;
#[path = "vm_trait/traits.rs"]
mod traits;

pub use errors::VmError;
pub use events::{LogStream, StopMode, VmEvent, VmExit, VmMetric};
pub use http::{
    BoxedPrivateServiceConnection, PrivateHttpRequest, PrivateHttpResponse,
    PrivateMailboxPublication, PrivateServiceConnection,
};
pub use spec::{
    DiskFormat, GuestCommand, NetworkMode, PortForward, PortProtocol, PrivateHttpServiceSpec,
    RUNTIME_AUTHORITY_CREDENTIAL_BYTES, RUNTIME_GIT_CREDENTIAL_BYTES, RootFilesystem,
    RuntimeAuthorityBootstrap, RuntimeGitBridge, VmDisk, VmId, VmMount, VmResources, VmSpec,
};
pub use traits::{VmInstance, VmProvider};

#[cfg(test)]
#[path = "vm_trait/tests.rs"]
mod tests;
