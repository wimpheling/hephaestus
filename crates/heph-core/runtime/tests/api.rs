//! Compile-time coverage for the approved `heph-runtime` facade surface.

use heph_runtime::{
    ArtifactId, DisabledWorkspaceManager, DiskFormat, GuestCommand, NetworkMode,
    PreparedRuntimeGitWorkspace, PreparedWorkspace, ResultArtifactMetadata, ResultId,
    ResultRepository, RootFilesystem, RunWorkspaceManager, RuntimeGitWorkspaceManager,
    RuntimeGitWorkspaceRequest, StopMode, VmError, VmId, VmInstance, VmProvider, VmSpec, Volume,
    VolumeAttachment, VolumeError, VolumeLease, VolumeMetadataRepository, VolumeStore,
    WorkspaceError, WorkspaceId, WorkspaceMetadata, WorkspaceMetadataRepository,
};
use std::sync::Arc;

const fn assert_type<T>() {}

#[test]
fn approved_runtime_contracts_compile() {
    assert_type::<VmId>();
    assert_type::<VmSpec>();
    assert_type::<DiskFormat>();
    assert_type::<GuestCommand>();
    assert_type::<NetworkMode>();
    assert_type::<RootFilesystem>();
    assert_type::<StopMode>();
    assert_type::<VmError>();
    assert_type::<Arc<dyn VmProvider>>();
    assert_type::<Arc<dyn VmInstance>>();
    assert_type::<Volume>();
    assert_type::<VolumeAttachment>();
    assert_type::<VolumeError>();
    assert_type::<VolumeLease>();
    assert_type::<Arc<dyn VolumeMetadataRepository>>();
    assert_type::<Arc<dyn VolumeStore>>();
    assert_type::<ArtifactId>();
    assert_type::<ResultId>();
    assert_type::<WorkspaceId>();
    assert_type::<WorkspaceMetadata>();
    assert_type::<WorkspaceError>();
    assert_type::<RuntimeGitWorkspaceRequest>();
    assert_type::<PreparedWorkspace>();
    assert_type::<PreparedRuntimeGitWorkspace>();
    assert_type::<ResultArtifactMetadata>();
    assert_type::<Arc<dyn ResultRepository>>();
    assert_type::<Arc<dyn WorkspaceMetadataRepository>>();
    assert_type::<Arc<dyn RunWorkspaceManager>>();
    assert_type::<Arc<dyn RuntimeGitWorkspaceManager>>();

    let _ = DisabledWorkspaceManager;
}
