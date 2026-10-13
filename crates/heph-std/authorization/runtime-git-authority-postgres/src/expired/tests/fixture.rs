//! Closed model inputs for pure denial controls, never persisted provenance.

use super::super::scope::{ReleaseRequirement, Shape, StoredBinding, StoredSnapshot};
use capability_domain::{
    AuthorizationSnapshot, AuthorizationSnapshotId, CapabilityBinding, CapabilityBindingId,
    CapabilityOperation, CapabilityRequirement, CapabilityRequirementId, CapabilityResource,
    CapabilityResourceKind, CapabilitySlotKey, RuntimeCredentialGeneration, RuntimeCredentialHash,
    RuntimeInvocation, RuntimeSessionId, RuntimeSessionIdentity, RuntimeSessionStatus,
    WorkloadKind, WorkloadPrincipal,
};
use git_capability_domain::{BoundGitCapability, RepositoryId};
use runtime_authority::{ExpiredRuntimeHandoff, LiveRunSnapshotPins, StoredRuntimeSession};
use time::OffsetDateTime;
use uuid::Uuid;

pub(super) fn shape() -> Shape {
    Shape {
        grammar_version: i16::try_from(git_capability_domain::GRAMMAR_VERSION).unwrap(),
        git_operations: vec!["discover".into(), "fetch".into()],
        ref_globs: vec!["refs/heads/main".into()],
        changed_path_globs: vec![],
        branch_update_policy: "fast_forward_only".into(),
        branch_create: false,
        branch_delete: false,
        tag_create: false,
        tag_update: false,
        tag_delete: false,
        other_create: false,
        other_update: false,
        other_delete: false,
        request_bytes: 1024,
        pack_bytes: 1024,
        object_count: 10,
        ref_updates: 1,
        exact_parent_required: false,
        normalized_hash: vec![0; 32],
    }
}

fn original() -> ExpiredRuntimeHandoff {
    let pins = LiveRunSnapshotPins::new(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        Uuid::from_u128(3),
        Uuid::from_u128(4),
        Uuid::from_u128(5),
        Uuid::from_u128(6),
    )
    .unwrap();
    let requirement_id = Uuid::from_u128(10);
    let repository_id = Uuid::from_u128(11);
    let binding_id = Uuid::from_u128(12);
    let requirement = CapabilityRequirement::new(
        CapabilityRequirementId::from_uuid(requirement_id),
        CapabilitySlotKey::parse("source").unwrap(),
        CapabilityResourceKind::Repository,
        [CapabilityOperation::GitRead],
        [],
        true,
    )
    .unwrap();
    let generic_binding = CapabilityBinding::bind(
        CapabilityBindingId::from_uuid(binding_id),
        &requirement,
        CapabilityResource::new(CapabilityResourceKind::Repository, repository_id),
        [CapabilityOperation::GitRead],
    )
    .unwrap();
    let generic_snapshot = AuthorizationSnapshot::new(
        AuthorizationSnapshotId::from_uuid(Uuid::from_u128(7)),
        WorkloadPrincipal::new(
            WorkloadKind::AgentInstance,
            pins.instance(),
            pins.revision(),
        ),
        "controlled-model-only",
        vec![generic_binding],
    )
    .unwrap();
    let issued_at = OffsetDateTime::UNIX_EPOCH;
    let expires_at = issued_at + time::Duration::seconds(60);
    let identity = RuntimeSessionIdentity::new(
        RuntimeSessionId::from_uuid(Uuid::from_u128(8)),
        generic_snapshot.principal(),
        RuntimeInvocation::Run(pins.run().into()),
        &generic_snapshot,
        issued_at,
        expires_at,
    )
    .unwrap();
    let session = StoredRuntimeSession {
        id: identity.id(),
        snapshot_id: generic_snapshot.id(),
        identity_hash: identity.normalized_hash(),
        generation: RuntimeCredentialGeneration::INITIAL,
        status: RuntimeSessionStatus::Expired,
        issued_at,
        expires_at,
        acknowledged_at: None,
        revoked_at: None,
    };
    ExpiredRuntimeHandoff::new(
        session,
        generic_snapshot,
        identity,
        pins,
        None,
        RuntimeCredentialHash::from_bytes([13; 32]),
        expires_at,
    )
    .unwrap()
}

pub(super) fn inputs() -> (
    ExpiredRuntimeHandoff,
    StoredSnapshot,
    StoredBinding,
    ReleaseRequirement,
) {
    let original = original();
    let pins = original.pins();
    let requirement_id = Uuid::from_u128(10);
    let repository_id = Uuid::from_u128(11);
    let binding_id = Uuid::from_u128(12);
    let mut release_shape = shape();
    let ceiling = release_shape.decode().unwrap();
    release_shape.normalized_hash = ceiling.normalized_hash().unwrap().as_bytes().to_vec();
    let mut binding_shape = shape();
    binding_shape.normalized_hash =
        BoundGitCapability::new(RepositoryId::new(repository_id), ceiling.clone(), &ceiling)
            .unwrap()
            .normalized_hash()
            .unwrap()
            .as_bytes()
            .to_vec();
    let snapshot = StoredSnapshot {
        snapshot_id: original.snapshot().id().as_uuid(),
        instance_revision_id: pins.revision(),
        binding_id,
        repository_id,
        expected_parent: None,
        shape: binding_shape.clone(),
    };
    let binding = StoredBinding {
        binding_id,
        instance_revision_id: pins.revision(),
        requirement_id,
        shape: binding_shape,
    };
    let release = ReleaseRequirement {
        requirement_id,
        release_agent_id: pins.release_agent(),
        requirement_hash: original
            .snapshot()
            .bindings()
            .next()
            .unwrap()
            .requirement_hash()
            .as_bytes()
            .to_vec(),
        declared_operations: vec!["git_read".into()],
        resource_kind: "repository".into(),
        shape: release_shape,
    };
    (original, snapshot, binding, release)
}
