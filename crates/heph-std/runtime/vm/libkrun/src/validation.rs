pub const PROVIDER_NAME: &str = "libkrun";
// libkrun's virtio-fs tag buffer is fixed at 36 bytes.
const MAX_VIRTIO_FS_TAG_BYTES: usize = 36;

mod config;
mod helpers;
mod policies;
mod spec;
#[cfg(test)]
#[path = "validation/tests/mod.rs"]
mod tests;
mod types;

pub use config::validate_config;
pub use helpers::validate_id;
pub use spec::prepare_spec;
pub use types::{PreparedForward, PreparedMount, PreparedNetwork, PreparedRoot, PreparedSpec};

// These facade types are consumed by downstream provider code rather than this crate internally.
#[allow(unused_imports)]
pub use types::{
    PreparedCommand, PreparedDisk, PreparedPrivateHttpService, PreparedRuntimeAuthority,
    PreparedRuntimeGitBridge,
};
