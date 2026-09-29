use super::{
    AuthorizationSnapshot, AuthorizationSnapshotId, CapabilityBinding, CapabilityBindingId,
    CapabilityError, CapabilityOperation, CapabilityRequirement, CapabilityRequirementId,
    CapabilityResource, CapabilityResourceKind, CapabilitySlotKey, GatewayInvocationId,
    RuntimeAuthority, RuntimeCredential, RuntimeCredentialGeneration, RuntimeInvocation,
    RuntimeSessionId, RuntimeSessionIdentity, RuntimeSessionStatus, WorkloadKind,
    WorkloadPrincipal,
};
use runtime_types::RunId;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const REQUIREMENT_ID: Uuid = Uuid::from_u128(1);
const BINDING_ID: Uuid = Uuid::from_u128(2);
const SNAPSHOT_ID: Uuid = Uuid::from_u128(3);
const SESSION_ID: Uuid = Uuid::from_u128(4);
const WORKLOAD_ID: Uuid = Uuid::from_u128(5);
const REVISION_ID: Uuid = Uuid::from_u128(6);
const RESOURCE_ID: Uuid = Uuid::from_u128(7);

fn repository_requirement(
    required: Vec<CapabilityOperation>,
    optional: Vec<CapabilityOperation>,
) -> CapabilityRequirement {
    CapabilityRequirement::new(
        CapabilityRequirementId::from_uuid(REQUIREMENT_ID),
        CapabilitySlotKey::parse("source_repo").expect("slot"),
        CapabilityResourceKind::Repository,
        required,
        optional,
        true,
    )
    .expect("requirement")
}

fn repository_binding() -> CapabilityBinding {
    let requirement = repository_requirement(
        vec![CapabilityOperation::GitRead],
        vec![CapabilityOperation::UpdateRef],
    );
    CapabilityBinding::bind(
        CapabilityBindingId::from_uuid(BINDING_ID),
        &requirement,
        CapabilityResource::new(CapabilityResourceKind::Repository, RESOURCE_ID),
        [CapabilityOperation::GitRead],
    )
    .expect("binding")
}

fn agent_principal() -> WorkloadPrincipal {
    WorkloadPrincipal::new(WorkloadKind::AgentInstance, WORKLOAD_ID, REVISION_ID)
}

fn snapshot() -> AuthorizationSnapshot {
    AuthorizationSnapshot::new(
        AuthorizationSnapshotId::from_uuid(SNAPSHOT_ID),
        agent_principal(),
        "melange-v1",
        vec![repository_binding()],
    )
    .expect("snapshot")
}

#[test]
fn validates_symbolic_slot_keys_and_serde() {
    let key = CapabilitySlotKey::parse("results-repo_2").expect("valid key");
    let json = serde_json::to_string(&key).expect("serialize key");
    assert_eq!(json, "\"results-repo_2\"");
    assert_eq!(
        serde_json::from_str::<CapabilitySlotKey>(&json).expect("deserialize key"),
        key
    );
    for invalid in ["", "2repo", "Repo", "repo/path", &"a".repeat(65)] {
        assert!(
            CapabilitySlotKey::parse(invalid).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn normalizes_requirements_and_hashes_deterministically() {
    let left = repository_requirement(
        vec![CapabilityOperation::GitRead],
        vec![
            CapabilityOperation::UpdateRef,
            CapabilityOperation::CreateRef,
        ],
    );
    let right = repository_requirement(
        vec![CapabilityOperation::GitRead],
        vec![
            CapabilityOperation::CreateRef,
            CapabilityOperation::UpdateRef,
        ],
    );
    assert_eq!(left, right);
    assert_eq!(left.normalized_hash(), right.normalized_hash());
    assert_eq!(left.normalized_hash().to_string().len(), 64);
}

#[test]
fn requirement_deserialization_reapplies_domain_validation() {
    let requirement = repository_requirement(
        vec![CapabilityOperation::GitRead],
        vec![CapabilityOperation::UpdateRef],
    );
    let json = serde_json::to_string(&requirement).expect("serialize requirement");
    assert_eq!(
        serde_json::from_str::<CapabilityRequirement>(&json).expect("deserialize requirement"),
        requirement
    );

    let invalid = json.replace("\"git_read\"", "\"restore\"");
    assert!(
        serde_json::from_str::<CapabilityRequirement>(&invalid).is_err(),
        "illegal operation must not bypass validation"
    );
}

#[test]
fn rejects_duplicate_and_illegal_requirement_operations() {
    let duplicate = CapabilityRequirement::new(
        CapabilityRequirementId::new(),
        CapabilitySlotKey::parse("repo").expect("slot"),
        CapabilityResourceKind::Repository,
        [CapabilityOperation::GitRead, CapabilityOperation::GitRead],
        [],
        true,
    );
    assert_eq!(
        duplicate.expect_err("duplicate must fail"),
        CapabilityError::DuplicateOperation(CapabilityOperation::GitRead)
    );

    let illegal = CapabilityRequirement::new(
        CapabilityRequirementId::new(),
        CapabilitySlotKey::parse("repo").expect("slot"),
        CapabilityResourceKind::Repository,
        [CapabilityOperation::Restore],
        [],
        true,
    );
    assert!(matches!(
        illegal,
        Err(CapabilityError::IllegalOperation { .. })
    ));
}

#[test]
fn exact_binding_cannot_omit_or_broaden_declared_authority() {
    let requirement = repository_requirement(
        vec![CapabilityOperation::GitRead],
        vec![CapabilityOperation::UpdateRef],
    );
    let resource = CapabilityResource::new(CapabilityResourceKind::Repository, RESOURCE_ID);
    let missing = CapabilityBinding::bind(
        CapabilityBindingId::new(),
        &requirement,
        resource,
        [CapabilityOperation::UpdateRef],
    );
    assert_eq!(
        missing.expect_err("required operation must be present"),
        CapabilityError::MissingRequiredOperation(CapabilityOperation::GitRead)
    );

    let broadened = CapabilityBinding::bind(
        CapabilityBindingId::new(),
        &requirement,
        resource,
        [CapabilityOperation::GitRead, CapabilityOperation::DeleteRef],
    );
    assert_eq!(
        broadened.expect_err("undeclared operation must fail"),
        CapabilityError::UndeclaredOperation(CapabilityOperation::DeleteRef)
    );
}

#[test]
fn snapshot_order_and_hash_are_deterministic() {
    let first = repository_binding();
    let mut second_requirement = repository_requirement(
        vec![CapabilityOperation::GitRead],
        vec![CapabilityOperation::UpdateRef],
    );
    second_requirement.id = CapabilityRequirementId::from_uuid(Uuid::from_u128(8));
    second_requirement.slot = CapabilitySlotKey::parse("archive_repo").expect("slot");
    let second = CapabilityBinding::bind(
        CapabilityBindingId::from_uuid(Uuid::from_u128(9)),
        &second_requirement,
        CapabilityResource::new(CapabilityResourceKind::Repository, Uuid::from_u128(10)),
        [CapabilityOperation::GitRead],
    )
    .expect("binding");
    let left = AuthorizationSnapshot::new(
        AuthorizationSnapshotId::from_uuid(SNAPSHOT_ID),
        agent_principal(),
        "melange-v1",
        vec![first.clone(), second.clone()],
    )
    .expect("snapshot");
    let right = AuthorizationSnapshot::new(
        AuthorizationSnapshotId::from_uuid(SNAPSHOT_ID),
        agent_principal(),
        "melange-v1",
        vec![second, first],
    )
    .expect("snapshot");
    assert_eq!(left, right);
    assert_eq!(left.normalized_hash(), right.normalized_hash());
}

#[test]
fn snapshot_rejects_duplicate_slots() {
    let first = repository_binding();
    let mut second = first.clone();
    second.id = CapabilityBindingId::new();
    let duplicate = AuthorizationSnapshot::new(
        AuthorizationSnapshotId::new(),
        agent_principal(),
        "melange-v1",
        vec![first, second],
    );
    assert!(matches!(
        duplicate,
        Err(CapabilityError::DuplicateBindingSlot(_))
    ));
}

#[test]
fn runtime_authority_requires_exact_identity_and_bounds_calls() {
    let snapshot = snapshot();
    let issued_at = OffsetDateTime::UNIX_EPOCH;
    let expires_at = issued_at + Duration::minutes(5);
    let identity = RuntimeSessionIdentity::new(
        RuntimeSessionId::from_uuid(SESSION_ID),
        agent_principal(),
        RuntimeInvocation::Run(RunId::from_uuid(Uuid::from_u128(11))),
        &snapshot,
        issued_at,
        expires_at,
    )
    .expect("identity");
    let authority = RuntimeAuthority::new(identity, snapshot).expect("authority");
    let repository = CapabilityResource::new(CapabilityResourceKind::Repository, RESOURCE_ID);
    assert!(authority.permits_at(issued_at, repository, CapabilityOperation::GitRead));
    assert!(!authority.permits_at(issued_at, repository, CapabilityOperation::UpdateRef));
    assert!(!authority.permits_at(expires_at, repository, CapabilityOperation::GitRead));
}

#[test]
fn invocation_kind_must_match_workload_kind() {
    let snapshot = snapshot();
    let invalid = RuntimeSessionIdentity::new(
        RuntimeSessionId::new(),
        agent_principal(),
        RuntimeInvocation::Gateway(GatewayInvocationId::new()),
        &snapshot,
        OffsetDateTime::UNIX_EPOCH,
        OffsetDateTime::UNIX_EPOCH + Duration::minutes(1),
    );
    assert_eq!(
        invalid.expect_err("gateway invocation cannot identify an agent run"),
        CapabilityError::InvocationKindMismatch
    );
}

#[test]
fn runtime_credential_is_redacted_and_bound_to_session_generation() {
    let credential = RuntimeCredential::from_secret([0x5a; 32]);
    let generation = RuntimeCredentialGeneration::INITIAL;
    let session_id = RuntimeSessionId::from_uuid(SESSION_ID);
    let verifier = credential.storage_hash(session_id, generation);

    assert!(verifier.verifies(&credential, session_id, generation));
    assert!(!verifier.verifies(&credential, RuntimeSessionId::new(), generation));
    assert!(!verifier.verifies(
        &credential,
        session_id,
        RuntimeCredentialGeneration::new(2).expect("positive generation")
    ));
    assert_eq!(credential.to_string(), "[REDACTED]");
    assert_eq!(
        serde_json::to_string(&credential).expect("redacted serialization"),
        "\"[REDACTED]\""
    );
    assert!(!format!("{credential:?}").contains("5a"));
    assert!(!format!("{verifier:?}").contains("5a"));
}

#[test]
fn runtime_session_lifecycle_is_monotonic() {
    assert!(RuntimeSessionStatus::PendingHandoff.can_transition_to(RuntimeSessionStatus::Active));
    assert!(RuntimeSessionStatus::Active.can_transition_to(RuntimeSessionStatus::Revoked));
    assert!(!RuntimeSessionStatus::Revoked.can_transition_to(RuntimeSessionStatus::Active));
    assert_eq!(
        RuntimeCredentialGeneration::new(0),
        Err(CapabilityError::InvalidCredentialGeneration)
    );
}
