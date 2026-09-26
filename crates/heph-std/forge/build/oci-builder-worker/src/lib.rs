//! Isolated, durable OCI production and rootfs materialization.
//!
//! This crate has no database or RPC dependency. It consumes claimed durable
//! jobs, materializes one exact source tree, invokes a rootless OCI image
//! with no network or credentials, and commits outcomes through its job port.

#[path = "oci-builder-worker/buildah.rs"]
mod buildah;
#[path = "oci-builder-worker/materialization.rs"]
mod materialization;
#[path = "oci-builder-worker/policy.rs"]
mod policy;
#[path = "oci-builder-worker/production.rs"]
mod production;
#[path = "oci-builder-worker/types.rs"]
mod types;

#[cfg(test)]
#[path = "oci-builder-worker/tests.rs"]
mod tests;

pub use buildah::{BuildahEngine, PublishedBuildahEngine, local_image_name};
pub use materialization::RootfsMaterializationWorker;
#[cfg(test)]
pub(crate) use materialization::install_guest_init;
pub use policy::DockerfilePolicy;
pub use production::OciImageProductionWorker;
pub use types::{
    IsolatedOciBuild, OciBuildEngine, OciOutputPublisher, OciRootfsExporter, PreparedSource,
    RegistryPublisherTokenIssuer, SourceCheckoutProvider,
};

pub use heph_build::{
    ClaimedMaterializationJob, ClaimedProductionJob, MaterializedRoot, OciImageProductionJobStore,
    OciImageProductionOutput, OciWorkerError, OciWorkerStoreError, RepositoryOciImageProvenance,
    RepositoryOciImagePublicationLease, RepositoryOciImagePublicationStore,
    RepositoryOciImageSourcePath,
};
