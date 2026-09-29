//! Compile-time coverage for the approved `heph-forge` facade surface.

use heph_forge::{
    AgentConfigRevisionId, CommitSha, CreateRepository, ForgeOutboxStore, ForgeRepositoryError,
    GitRef, GitValueError, OrganizationId, OutboxRecord, Project, ProjectId, ReceiveId,
    ReceiveResult, RefUpdate, Repository, RepositoryId, RunRequest, RunRequestId,
    RuntimeReceiveProvenance,
};
use std::sync::Arc;

const fn assert_type<T>() {}

#[test]
fn approved_forge_contracts_compile() {
    assert_type::<OrganizationId>();
    assert_type::<ProjectId>();
    assert_type::<RepositoryId>();
    assert_type::<ReceiveId>();
    assert_type::<AgentConfigRevisionId>();
    assert_type::<RunRequestId>();
    assert_type::<GitRef>();
    assert_type::<CommitSha>();
    assert_type::<GitValueError>();
    assert_type::<Project>();
    assert_type::<Repository>();
    assert_type::<RefUpdate>();
    assert_type::<RuntimeReceiveProvenance>();
    assert_type::<CreateRepository>();
    assert_type::<ReceiveResult>();
    assert_type::<RunRequest>();
    assert_type::<OutboxRecord>();
    assert_type::<ForgeRepositoryError>();
    assert_type::<Arc<dyn ForgeOutboxStore>>();
}
