//! Compile-time coverage for the provider-neutral OCI build contracts.

use heph_build::{
    ClaimedMaterializationJob, ClaimedProductionJob, MaterializedRoot, OciImageProductionJobStore,
    OciImageProductionOutput, OciWorkerError, OciWorkerStoreError, RepositoryOciImageProvenance,
    RepositoryOciImagePublicationLease, RepositoryOciImagePublicationStore,
    RepositoryOciImageSourcePath,
};
use std::sync::Arc;

const fn assert_type<T>() {}

fn error_conversion_is_available() {
    let _: fn(OciWorkerStoreError) -> OciWorkerError = OciWorkerError::Store;
}

#[test]
fn shared_ports_and_dtos_are_available_from_the_facade() {
    assert_type::<ClaimedProductionJob>();
    assert_type::<ClaimedMaterializationJob>();
    assert_type::<OciImageProductionOutput>();
    assert_type::<MaterializedRoot>();
    assert_type::<RepositoryOciImageProvenance>();
    assert_type::<RepositoryOciImageSourcePath>();
    assert_type::<Arc<dyn OciImageProductionJobStore>>();
    assert_type::<Arc<dyn RepositoryOciImagePublicationStore>>();
    assert_type::<RepositoryOciImagePublicationLease>();
    assert_type::<OciWorkerError>();
    assert_type::<OciWorkerStoreError>();
    error_conversion_is_available();
}
