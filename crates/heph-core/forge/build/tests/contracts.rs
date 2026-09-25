//! Compile-time coverage for the provider-neutral OCI build contracts.

use heph_build::{
    ClaimedMaterializationJob, ClaimedProductionJob, MaterializedRoot, OciImageProductionJobStore,
    OciImageProductionOutput, OciWorkerError, OciWorkerStoreError, RepositoryOciImageProvenance,
    RepositoryOciImagePublicationLease, RepositoryOciImagePublicationStore,
    RepositoryOciImageSourcePath,
};
use std::sync::Arc;

fn shared_contracts(
    _job: ClaimedProductionJob,
    _materialization: ClaimedMaterializationJob,
    _output: OciImageProductionOutput,
    _root: MaterializedRoot,
    _provenance: RepositoryOciImageProvenance,
    _path: RepositoryOciImageSourcePath,
    _store: Arc<dyn OciImageProductionJobStore>,
    _publication: Arc<dyn RepositoryOciImagePublicationStore>,
) -> Result<RepositoryOciImagePublicationLease, OciWorkerError> {
    let _: fn(OciWorkerStoreError) -> OciWorkerError = OciWorkerError::Store;
    Err(OciWorkerStoreError::Conflict.into())
}

#[test]
fn shared_ports_and_dtos_are_available_from_the_facade() {
    let _: fn(
        ClaimedProductionJob,
        ClaimedMaterializationJob,
        OciImageProductionOutput,
        MaterializedRoot,
        RepositoryOciImageProvenance,
        RepositoryOciImageSourcePath,
        Arc<dyn OciImageProductionJobStore>,
        Arc<dyn RepositoryOciImagePublicationStore>,
    ) -> Result<RepositoryOciImagePublicationLease, OciWorkerError> = shared_contracts;
}
