//! Provider-neutral capability requirements, bindings, and runtime ceilings.
//!
//! These values describe immutable authority. Storage adapters and live
//! authorization providers remain responsible for persistence, revocation,
//! tenant boundaries, and current allow/deny decisions.

use runtime_types::RunId;
use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt, str::FromStr};
use time::OffsetDateTime;
use uuid::Uuid;

/// Maximum length of a symbolic capability slot key.
pub const MAX_CAPABILITY_SLOT_KEY_BYTES: usize = 64;
/// Maximum number of operations declared by or granted to one slot.
pub const MAX_OPERATIONS_PER_CAPABILITY: usize = 32;
/// Size of a canonical authority hash in bytes.
pub const AUTHORITY_HASH_BYTES: usize = 32;
/// Size of a runtime bearer credential in bytes.
pub const RUNTIME_CREDENTIAL_BYTES: usize = 32;

const RUNTIME_CREDENTIAL_HASH_DOMAIN: &[u8] = b"hephaestus.runtime-credential-verifier.v1\0";
const REDACTED: &str = "[REDACTED]";

macro_rules! identifier {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a random version 4 identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Creates an identifier from its UUID representation.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the UUID representation.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

identifier!(
    CapabilityRequirementId,
    "A stable identifier for one symbolic release capability requirement."
);
identifier!(
    CapabilityBindingId,
    "A stable identifier for one exact immutable capability binding."
);
identifier!(
    AuthorizationSnapshotId,
    "A stable identifier for one immutable dispatch-time authority snapshot."
);
identifier!(
    RuntimeSessionId,
    "A stable identifier for one short-lived authenticated runtime session."
);
identifier!(
    GatewayInvocationId,
    "A stable identifier for one exact gateway HTTP invocation."
);

/// A stable symbolic capability name declared by released code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CapabilitySlotKey(String);

impl CapabilitySlotKey {
    /// Parses a bounded lowercase key.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError::InvalidSlotKey`] unless the value starts
    /// with a lowercase ASCII letter and contains only lowercase letters,
    /// digits, underscores, or hyphens.
    pub fn parse(value: impl Into<String>) -> Result<Self, CapabilityError> {
        let value = value.into();
        let mut bytes = value.bytes();
        let valid = !value.is_empty()
            && value.len() <= MAX_CAPABILITY_SLOT_KEY_BYTES
            && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            && bytes.all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
            });
        if !valid {
            return Err(CapabilityError::InvalidSlotKey);
        }
        Ok(Self(value))
    }

    /// Returns the validated key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CapabilitySlotKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for CapabilitySlotKey {
    type Error = CapabilityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<CapabilitySlotKey> for String {
    fn from(value: CapabilitySlotKey) -> Self {
        value.0
    }
}

/// Resource categories which may be selected for a capability slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityResourceKind {
    /// A project-owned Git repository.
    Repository,
    /// A project boundary.
    Project,
    /// A durable agent instance.
    AgentInstance,
    /// A declared HTTP gateway.
    Gateway,
    /// One exact execution.
    Run,
    /// A persistent private state volume.
    StateVolume,
}

impl CapabilityResourceKind {
    /// Returns the stable wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::Project => "project",
            Self::AgentInstance => "agent_instance",
            Self::Gateway => "gateway",
            Self::Run => "run",
            Self::StateVolume => "state_volume",
        }
    }
}

/// Closed semantic operation vocabulary for capability ceilings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOperation {
    /// Inspect resource metadata.
    Inspect,
    /// Change resource configuration.
    Configure,
    /// Execute or invoke the resource.
    Execute,
    /// Apply an update to the resource.
    Update,
    /// Pause new work.
    Pause,
    /// Recover paused or failed work.
    Recover,
    /// Cancel an exact execution.
    Cancel,
    /// Attach a resource to a workload.
    Attach,
    /// Restore an earlier durable state.
    Restore,
    /// Fetch raw Git refs and objects.
    GitRead,
    /// Create a Git ref.
    CreateRef,
    /// Fast-forward an existing Git ref.
    UpdateRef,
    /// Non-fast-forward update of an existing Git ref.
    ForceUpdateRef,
    /// Delete a Git ref.
    DeleteRef,
    /// Create a Git tag.
    CreateTag,
    /// Delete a Git tag.
    DeleteTag,
    /// Trigger execution from an accepted repository update.
    TriggerRun,
    /// Manage repository/ref attachments.
    ManageAttachments,
}

impl CapabilityOperation {
    /// Returns the stable wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::Configure => "configure",
            Self::Execute => "execute",
            Self::Update => "update",
            Self::Pause => "pause",
            Self::Recover => "recover",
            Self::Cancel => "cancel",
            Self::Attach => "attach",
            Self::Restore => "restore",
            Self::GitRead => "git_read",
            Self::CreateRef => "create_ref",
            Self::UpdateRef => "update_ref",
            Self::ForceUpdateRef => "force_update_ref",
            Self::DeleteRef => "delete_ref",
            Self::CreateTag => "create_tag",
            Self::DeleteTag => "delete_tag",
            Self::TriggerRun => "trigger_run",
            Self::ManageAttachments => "manage_attachments",
        }
    }

    /// Returns whether the operation is defined for the resource category.
    #[must_use]
    pub const fn is_legal_for(self, resource_kind: CapabilityResourceKind) -> bool {
        use CapabilityOperation::{
            Attach, Cancel, Configure, CreateRef, CreateTag, DeleteRef, DeleteTag, Execute,
            ForceUpdateRef, GitRead, Inspect, ManageAttachments, Pause, Recover, Restore,
            TriggerRun, Update, UpdateRef,
        };
        use CapabilityResourceKind::{
            AgentInstance, Gateway, Project, Repository, Run, StateVolume,
        };

        match resource_kind {
            Repository => matches!(
                self,
                Inspect
                    | GitRead
                    | CreateRef
                    | UpdateRef
                    | ForceUpdateRef
                    | DeleteRef
                    | CreateTag
                    | DeleteTag
                    | TriggerRun
                    | ManageAttachments
            ),
            Project | AgentInstance | Gateway => {
                matches!(
                    self,
                    Inspect | Configure | Execute | Update | Pause | Recover
                )
            }
            Run => matches!(self, Inspect | Cancel | Recover),
            StateVolume => matches!(self, Inspect | Attach | Restore),
        }
    }
}

/// A symbolic release request for one kind of controlled resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "CapabilityRequirementWire")]
pub struct CapabilityRequirement {
    id: CapabilityRequirementId,
    slot: CapabilitySlotKey,
    resource_kind: CapabilityResourceKind,
    required_operations: BTreeSet<CapabilityOperation>,
    optional_operations: BTreeSet<CapabilityOperation>,
    slot_required: bool,
}

impl CapabilityRequirement {
    /// Validates and constructs one normalized requirement.
    ///
    /// # Errors
    ///
    /// Rejects empty declarations, oversized declarations, illegal resource
    /// operations, duplicate input operations, and operations present in both
    /// the required and optional sets.
    pub fn new(
        id: CapabilityRequirementId,
        slot: CapabilitySlotKey,
        resource_kind: CapabilityResourceKind,
        required_operations: impl IntoIterator<Item = CapabilityOperation>,
        optional_operations: impl IntoIterator<Item = CapabilityOperation>,
        slot_required: bool,
    ) -> Result<Self, CapabilityError> {
        let required_operations = operation_set(required_operations)?;
        let optional_operations = operation_set(optional_operations)?;
        if required_operations.is_empty() && optional_operations.is_empty() {
            return Err(CapabilityError::EmptyOperationSet);
        }
        validate_operations(resource_kind, &required_operations)?;
        validate_operations(resource_kind, &optional_operations)?;
        if let Some(operation) = required_operations
            .intersection(&optional_operations)
            .next()
            .copied()
        {
            return Err(CapabilityError::OperationRequiredAndOptional(operation));
        }
        Ok(Self {
            id,
            slot,
            resource_kind,
            required_operations,
            optional_operations,
            slot_required,
        })
    }

    /// Returns the stable requirement identifier.
    #[must_use]
    pub const fn id(&self) -> CapabilityRequirementId {
        self.id
    }

    /// Returns the symbolic release slot.
    #[must_use]
    pub const fn slot(&self) -> &CapabilitySlotKey {
        &self.slot
    }

    /// Returns the required resource category.
    #[must_use]
    pub const fn resource_kind(&self) -> CapabilityResourceKind {
        self.resource_kind
    }

    /// Returns required operations in normalized order.
    #[must_use]
    pub fn required_operations(&self) -> impl ExactSizeIterator<Item = CapabilityOperation> + '_ {
        self.required_operations.iter().copied()
    }

    /// Returns optional operations in normalized order.
    #[must_use]
    pub fn optional_operations(&self) -> impl ExactSizeIterator<Item = CapabilityOperation> + '_ {
        self.optional_operations.iter().copied()
    }

    /// Returns whether the slot itself must be bound before dispatch.
    #[must_use]
    pub const fn slot_required(&self) -> bool {
        self.slot_required
    }

    /// Returns whether the operation is inside the declared ceiling.
    #[must_use]
    pub fn declares(&self, operation: CapabilityOperation) -> bool {
        self.required_operations.contains(&operation)
            || self.optional_operations.contains(&operation)
    }

    /// Returns a stable hash of the normalized requirement.
    #[must_use]
    pub fn normalized_hash(&self) -> AuthorityHash {
        let mut hasher = CanonicalHasher::new(b"hephaestus.capability-requirement.v1");
        hasher.uuid(self.id.as_uuid());
        hasher.text(self.slot.as_str());
        hasher.text(self.resource_kind.as_str());
        hasher.operations(&self.required_operations);
        hasher.operations(&self.optional_operations);
        hasher.boolean(self.slot_required);
        hasher.finish()
    }
}

#[derive(Deserialize)]
struct CapabilityRequirementWire {
    id: CapabilityRequirementId,
    slot: CapabilitySlotKey,
    resource_kind: CapabilityResourceKind,
    required_operations: Vec<CapabilityOperation>,
    optional_operations: Vec<CapabilityOperation>,
    slot_required: bool,
}

impl TryFrom<CapabilityRequirementWire> for CapabilityRequirement {
    type Error = CapabilityError;

    fn try_from(value: CapabilityRequirementWire) -> Result<Self, Self::Error> {
        Self::new(
            value.id,
            value.slot,
            value.resource_kind,
            value.required_operations,
            value.optional_operations,
            value.slot_required,
        )
    }
}

/// One exact selected Hephaestus resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CapabilityResource {
    /// Controlled resource category.
    pub kind: CapabilityResourceKind,
    /// Stable exact resource identifier.
    pub id: Uuid,
}

impl CapabilityResource {
    /// Creates an exact typed resource reference.
    #[must_use]
    pub const fn new(kind: CapabilityResourceKind, id: Uuid) -> Self {
        Self { kind, id }
    }
}

/// One immutable binding from a symbolic slot to an exact resource ceiling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapabilityBinding {
    id: CapabilityBindingId,
    requirement_id: CapabilityRequirementId,
    requirement_hash: AuthorityHash,
    slot: CapabilitySlotKey,
    resource: CapabilityResource,
    granted_operations: BTreeSet<CapabilityOperation>,
}

impl CapabilityBinding {
    /// Validates a selected resource and explicit grants against a requirement.
    ///
    /// # Errors
    ///
    /// Rejects incompatible resource categories, missing required operations,
    /// undeclared operations, duplicate operations, and empty grants.
    pub fn bind(
        id: CapabilityBindingId,
        requirement: &CapabilityRequirement,
        resource: CapabilityResource,
        granted_operations: impl IntoIterator<Item = CapabilityOperation>,
    ) -> Result<Self, CapabilityError> {
        if resource.kind != requirement.resource_kind {
            return Err(CapabilityError::IncompatibleResourceKind {
                expected: requirement.resource_kind,
                actual: resource.kind,
            });
        }
        let granted_operations = operation_set(granted_operations)?;
        if granted_operations.is_empty() {
            return Err(CapabilityError::EmptyOperationSet);
        }
        validate_operations(resource.kind, &granted_operations)?;
        if let Some(operation) = requirement
            .required_operations
            .difference(&granted_operations)
            .next()
            .copied()
        {
            return Err(CapabilityError::MissingRequiredOperation(operation));
        }
        if let Some(operation) = granted_operations
            .iter()
            .find(|operation| !requirement.declares(**operation))
            .copied()
        {
            return Err(CapabilityError::UndeclaredOperation(operation));
        }
        Ok(Self {
            id,
            requirement_id: requirement.id,
            requirement_hash: requirement.normalized_hash(),
            slot: requirement.slot.clone(),
            resource,
            granted_operations,
        })
    }

    /// Returns the binding identifier.
    #[must_use]
    pub const fn id(&self) -> CapabilityBindingId {
        self.id
    }

    /// Returns the bound requirement identifier.
    #[must_use]
    pub const fn requirement_id(&self) -> CapabilityRequirementId {
        self.requirement_id
    }

    /// Returns the hash of the exact requirement used during binding.
    #[must_use]
    pub const fn requirement_hash(&self) -> AuthorityHash {
        self.requirement_hash
    }

    /// Returns the symbolic slot.
    #[must_use]
    pub const fn slot(&self) -> &CapabilitySlotKey {
        &self.slot
    }

    /// Returns the exact bound resource.
    #[must_use]
    pub const fn resource(&self) -> CapabilityResource {
        self.resource
    }

    /// Returns granted operations in normalized order.
    #[must_use]
    pub fn granted_operations(&self) -> impl ExactSizeIterator<Item = CapabilityOperation> + '_ {
        self.granted_operations.iter().copied()
    }

    /// Returns whether this immutable binding grants an operation.
    #[must_use]
    pub fn grants(&self, operation: CapabilityOperation) -> bool {
        self.granted_operations.contains(&operation)
    }

    /// Returns a stable hash of the normalized exact binding.
    #[must_use]
    pub fn normalized_hash(&self) -> AuthorityHash {
        let mut hasher = CanonicalHasher::new(b"hephaestus.capability-binding.v1");
        hasher.uuid(self.id.as_uuid());
        hasher.uuid(self.requirement_id.as_uuid());
        hasher.bytes(self.requirement_hash.as_bytes());
        hasher.text(self.slot.as_str());
        hasher.text(self.resource.kind.as_str());
        hasher.uuid(self.resource.id);
        hasher.operations(&self.granted_operations);
        hasher.finish()
    }
}

/// Durable workload kind authenticated by a runtime session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    /// A durable agent instance.
    AgentInstance,
    /// A declared HTTP gateway.
    Gateway,
}

impl WorkloadKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::AgentInstance => "agent_instance",
            Self::Gateway => "gateway",
        }
    }
}

/// Exact immutable workload revision selected for dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkloadPrincipal {
    /// Workload category.
    pub kind: WorkloadKind,
    /// Durable workload identity.
    pub id: Uuid,
    /// Immutable selected revision identity.
    pub revision_id: Uuid,
}

impl WorkloadPrincipal {
    /// Creates an exact workload principal.
    #[must_use]
    pub const fn new(kind: WorkloadKind, id: Uuid, revision_id: Uuid) -> Self {
        Self {
            kind,
            id,
            revision_id,
        }
    }
}

/// Immutable dispatch-time maximum authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorizationSnapshot {
    id: AuthorizationSnapshotId,
    principal: WorkloadPrincipal,
    authorization_model_version: String,
    bindings: Vec<CapabilityBinding>,
    normalized_hash: AuthorityHash,
}

impl AuthorizationSnapshot {
    /// Creates a normalized immutable snapshot.
    ///
    /// # Errors
    ///
    /// Rejects an empty authorization model version, duplicate binding IDs,
    /// or more than one binding for the same symbolic slot.
    pub fn new(
        id: AuthorizationSnapshotId,
        principal: WorkloadPrincipal,
        authorization_model_version: impl Into<String>,
        mut bindings: Vec<CapabilityBinding>,
    ) -> Result<Self, CapabilityError> {
        let authorization_model_version = authorization_model_version.into();
        if authorization_model_version.is_empty() {
            return Err(CapabilityError::EmptyAuthorizationModelVersion);
        }
        bindings.sort_by(|left, right| left.slot.cmp(&right.slot));
        let mut binding_ids = BTreeSet::new();
        let mut slots = BTreeSet::new();
        for binding in &bindings {
            if !binding_ids.insert(binding.id) {
                return Err(CapabilityError::DuplicateBindingId(binding.id));
            }
            if !slots.insert(binding.slot.clone()) {
                return Err(CapabilityError::DuplicateBindingSlot(binding.slot.clone()));
            }
        }
        let normalized_hash = snapshot_hash(principal, &authorization_model_version, &bindings);
        Ok(Self {
            id,
            principal,
            authorization_model_version,
            bindings,
            normalized_hash,
        })
    }

    /// Returns the snapshot identifier.
    #[must_use]
    pub const fn id(&self) -> AuthorizationSnapshotId {
        self.id
    }

    /// Returns the exact workload revision whose bindings were snapshotted.
    #[must_use]
    pub const fn principal(&self) -> WorkloadPrincipal {
        self.principal
    }

    /// Returns the authorization model version used during resolution.
    #[must_use]
    pub fn authorization_model_version(&self) -> &str {
        &self.authorization_model_version
    }

    /// Returns normalized exact bindings.
    #[must_use]
    pub fn bindings(&self) -> impl ExactSizeIterator<Item = &CapabilityBinding> {
        self.bindings.iter()
    }

    /// Returns the immutable normalized snapshot hash.
    #[must_use]
    pub const fn normalized_hash(&self) -> AuthorityHash {
        self.normalized_hash
    }

    /// Returns whether the ceiling contains the exact operation and resource.
    #[must_use]
    pub fn allows(&self, resource: CapabilityResource, operation: CapabilityOperation) -> bool {
        self.bindings
            .iter()
            .any(|binding| binding.resource == resource && binding.grants(operation))
    }
}

/// One exact execution or gateway invocation authenticated by a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeInvocation {
    /// One durable agent run.
    Run(RunId),
    /// One bounded HTTP gateway invocation.
    Gateway(GatewayInvocationId),
}

impl RuntimeInvocation {
    fn canonicalize(self, hasher: &mut CanonicalHasher) {
        match self {
            Self::Run(id) => {
                hasher.text("run");
                hasher.uuid(id.as_uuid());
            }
            Self::Gateway(id) => {
                hasher.text("gateway");
                hasher.uuid(id.as_uuid());
            }
        }
    }
}

/// Immutable identity claims for one short-lived runtime session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeSessionIdentity {
    id: RuntimeSessionId,
    principal: WorkloadPrincipal,
    invocation: RuntimeInvocation,
    snapshot_id: AuthorizationSnapshotId,
    snapshot_hash: AuthorityHash,
    issued_at: OffsetDateTime,
    expires_at: OffsetDateTime,
}

impl RuntimeSessionIdentity {
    /// Creates exact bounded identity claims for a runtime session.
    ///
    /// # Errors
    ///
    /// Rejects a non-positive validity interval or an invocation whose kind
    /// does not match the durable workload principal.
    pub fn new(
        id: RuntimeSessionId,
        principal: WorkloadPrincipal,
        invocation: RuntimeInvocation,
        snapshot: &AuthorizationSnapshot,
        issued_at: OffsetDateTime,
        expires_at: OffsetDateTime,
    ) -> Result<Self, CapabilityError> {
        if expires_at <= issued_at {
            return Err(CapabilityError::InvalidSessionValidity);
        }
        if principal != snapshot.principal {
            return Err(CapabilityError::SnapshotPrincipalMismatch);
        }
        let invocation_matches = matches!(
            (principal.kind, invocation),
            (WorkloadKind::AgentInstance, RuntimeInvocation::Run(_))
                | (WorkloadKind::Gateway, RuntimeInvocation::Gateway(_))
        );
        if !invocation_matches {
            return Err(CapabilityError::InvocationKindMismatch);
        }
        Ok(Self {
            id,
            principal,
            invocation,
            snapshot_id: snapshot.id,
            snapshot_hash: snapshot.normalized_hash,
            issued_at,
            expires_at,
        })
    }

    /// Returns the runtime session identifier.
    #[must_use]
    pub const fn id(&self) -> RuntimeSessionId {
        self.id
    }

    /// Returns the exact workload revision principal.
    #[must_use]
    pub const fn principal(&self) -> WorkloadPrincipal {
        self.principal
    }

    /// Returns the exact execution or HTTP invocation.
    #[must_use]
    pub const fn invocation(&self) -> RuntimeInvocation {
        self.invocation
    }

    /// Returns the immutable authority snapshot identifier.
    #[must_use]
    pub const fn snapshot_id(&self) -> AuthorizationSnapshotId {
        self.snapshot_id
    }

    /// Returns the immutable authority snapshot hash.
    #[must_use]
    pub const fn snapshot_hash(&self) -> AuthorityHash {
        self.snapshot_hash
    }

    /// Returns the issue time.
    #[must_use]
    pub const fn issued_at(&self) -> OffsetDateTime {
        self.issued_at
    }

    /// Returns the exclusive expiry time.
    #[must_use]
    pub const fn expires_at(&self) -> OffsetDateTime {
        self.expires_at
    }

    /// Returns whether these claims are valid at the given instant.
    #[must_use]
    pub fn is_valid_at(&self, now: OffsetDateTime) -> bool {
        now >= self.issued_at && now < self.expires_at
    }

    /// Returns a stable hash of these exact identity claims.
    #[must_use]
    pub fn normalized_hash(&self) -> AuthorityHash {
        let mut hasher = CanonicalHasher::new(b"hephaestus.runtime-session-identity.v1");
        hasher.uuid(self.id.as_uuid());
        principal_canonicalize(self.principal, &mut hasher);
        self.invocation.canonicalize(&mut hasher);
        hasher.uuid(self.snapshot_id.as_uuid());
        hasher.bytes(self.snapshot_hash.as_bytes());
        hasher.timestamp(self.issued_at);
        hasher.timestamp(self.expires_at);
        hasher.finish()
    }
}

/// Stable positive generation for one runtime credential handoff.
///
/// A session starts at generation one. Redelivery uses that same generation;
/// rotating bearer material requires a different runtime session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeCredentialGeneration(u64);

impl RuntimeCredentialGeneration {
    /// The only generation currently issued for a new runtime session.
    pub const INITIAL: Self = Self(1);

    /// Constructs a positive generation.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError::InvalidCredentialGeneration`] for zero.
    pub const fn new(value: u64) -> Result<Self, CapabilityError> {
        if value == 0 {
            Err(CapabilityError::InvalidCredentialGeneration)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the stored integer representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Opaque runtime bearer material.
///
/// This value is deliberately neither cloneable nor deserializable. Durable
/// stores receive only [`RuntimeCredentialHash`].
pub struct RuntimeCredential([u8; RUNTIME_CREDENTIAL_BYTES]);

impl RuntimeCredential {
    /// Wraps 256 bits produced by a cryptographically secure random source.
    #[must_use]
    pub const fn from_secret(secret: [u8; RUNTIME_CREDENTIAL_BYTES]) -> Self {
        Self(secret)
    }

    /// Exposes bearer material only at the authenticated bootstrap boundary.
    #[must_use]
    pub const fn expose(&self) -> &[u8; RUNTIME_CREDENTIAL_BYTES] {
        &self.0
    }

    /// Derives the only representation permitted in durable storage.
    #[must_use]
    pub fn storage_hash(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> RuntimeCredentialHash {
        let mut digest = Sha256::new();
        digest.update(RUNTIME_CREDENTIAL_HASH_DOMAIN);
        digest.update(session_id.as_uuid().as_bytes());
        digest.update(generation.get().to_be_bytes());
        digest.update(self.0);
        RuntimeCredentialHash(digest.finalize().into())
    }
}

impl Drop for RuntimeCredential {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            *std::hint::black_box(byte) = 0;
        }
    }
}

impl fmt::Debug for RuntimeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeCredential([REDACTED])")
    }
}

impl fmt::Display for RuntimeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

impl Serialize for RuntimeCredential {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(REDACTED)
    }
}

/// Domain-separated one-way verifier for a runtime credential.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeCredentialHash([u8; 32]);

impl RuntimeCredentialHash {
    /// Reconstructs a verifier loaded from trusted durable storage.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the bytes safe for durable storage.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Checks candidate bearer material without an early exit on digest bytes.
    #[must_use]
    pub fn verifies(
        self,
        credential: &RuntimeCredential,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> bool {
        let candidate = credential.storage_hash(session_id, generation);
        self.0
            .iter()
            .zip(candidate.0.iter())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
    }
}

impl fmt::Debug for RuntimeCredentialHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeCredentialHash([REDACTED])")
    }
}

/// Lifecycle of one short-lived generic runtime session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSessionStatus {
    /// Credential exists in a host-only handoff envelope but is not acknowledged.
    PendingHandoff,
    /// The guest acknowledged the exact issuance generation.
    Active,
    /// Authority was permanently revoked.
    Revoked,
    /// The session reached its expiry.
    Expired,
}

impl RuntimeSessionStatus {
    /// Returns whether a lifecycle transition is valid.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::PendingHandoff,
                Self::Active | Self::Revoked | Self::Expired
            ) | (Self::Active, Self::Revoked | Self::Expired)
        )
    }
}

/// A verified runtime identity paired with its immutable authority ceiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAuthority {
    identity: RuntimeSessionIdentity,
    snapshot: AuthorizationSnapshot,
}

impl RuntimeAuthority {
    /// Pairs identity claims with the exact referenced snapshot.
    ///
    /// # Errors
    ///
    /// Rejects a different snapshot identifier, hash, or workload revision.
    pub fn new(
        identity: RuntimeSessionIdentity,
        snapshot: AuthorizationSnapshot,
    ) -> Result<Self, CapabilityError> {
        if identity.snapshot_id != snapshot.id
            || identity.snapshot_hash != snapshot.normalized_hash
            || identity.principal != snapshot.principal
        {
            return Err(CapabilityError::SnapshotIdentityMismatch);
        }
        Ok(Self { identity, snapshot })
    }

    /// Returns the exact authenticated runtime identity.
    #[must_use]
    pub const fn identity(&self) -> &RuntimeSessionIdentity {
        &self.identity
    }

    /// Returns the immutable authorization snapshot.
    #[must_use]
    pub const fn snapshot(&self) -> &AuthorizationSnapshot {
        &self.snapshot
    }

    /// Returns whether the request is inside the immutable ceiling and the
    /// runtime identity is temporally valid.
    ///
    /// A true result is not a live authorization decision. The caller must
    /// still check current authorization and resource lifecycle state.
    #[must_use]
    pub fn permits_at(
        &self,
        now: OffsetDateTime,
        resource: CapabilityResource,
        operation: CapabilityOperation,
    ) -> bool {
        self.identity.is_valid_at(now) && self.snapshot.allows(resource, operation)
    }
}

/// Stable SHA-256 digest of one normalized authority value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthorityHash([u8; AUTHORITY_HASH_BYTES]);

impl AuthorityHash {
    /// Reconstructs a hash loaded from trusted durable storage.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; AUTHORITY_HASH_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; AUTHORITY_HASH_BYTES] {
        &self.0
    }
}

impl fmt::Display for AuthorityHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// Invalid capability or runtime authority value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CapabilityError {
    /// A symbolic slot key was malformed or outside its bound.
    #[error(
        "capability slot key must start with a lowercase letter and contain 1 to 64 lowercase ASCII letters, digits, underscores, or hyphens"
    )]
    InvalidSlotKey,
    /// The input operation collection contained the same operation twice.
    #[error("duplicate capability operation {0:?}")]
    DuplicateOperation(CapabilityOperation),
    /// The operation set was empty.
    #[error("a capability must declare or grant at least one operation")]
    EmptyOperationSet,
    /// The operation set exceeded its maximum size.
    #[error("a capability may contain at most {MAX_OPERATIONS_PER_CAPABILITY} operations")]
    TooManyOperations,
    /// The operation is undefined for the selected resource category.
    #[error("operation {operation:?} is not legal for resource kind {resource_kind:?}")]
    IllegalOperation {
        /// Invalid operation.
        operation: CapabilityOperation,
        /// Selected resource category.
        resource_kind: CapabilityResourceKind,
    },
    /// One operation was classified as both required and optional.
    #[error("operation {0:?} cannot be both required and optional")]
    OperationRequiredAndOptional(CapabilityOperation),
    /// The selected resource category did not match the declared category.
    #[error("resource kind {actual:?} is incompatible with required kind {expected:?}")]
    IncompatibleResourceKind {
        /// Declared resource category.
        expected: CapabilityResourceKind,
        /// Selected resource category.
        actual: CapabilityResourceKind,
    },
    /// An explicit grant omitted a required operation.
    #[error("binding is missing required operation {0:?}")]
    MissingRequiredOperation(CapabilityOperation),
    /// An explicit grant exceeded the release declaration.
    #[error("binding grants undeclared operation {0:?}")]
    UndeclaredOperation(CapabilityOperation),
    /// A snapshot contained one binding identifier more than once.
    #[error("duplicate capability binding ID {0}")]
    DuplicateBindingId(CapabilityBindingId),
    /// A snapshot contained more than one binding for a symbolic slot.
    #[error("duplicate capability binding slot {0}")]
    DuplicateBindingSlot(CapabilitySlotKey),
    /// The authorization model version was empty.
    #[error("authorization model version cannot be empty")]
    EmptyAuthorizationModelVersion,
    /// The runtime session expiry did not follow its issue time.
    #[error("runtime session expiry must be later than its issue time")]
    InvalidSessionValidity,
    /// A runtime credential issuance generation was zero.
    #[error("runtime credential issuance generation must be positive")]
    InvalidCredentialGeneration,
    /// A runtime invocation kind did not match its workload principal kind.
    #[error("runtime invocation kind does not match workload principal kind")]
    InvocationKindMismatch,
    /// The snapshot belongs to a different workload revision.
    #[error("authorization snapshot belongs to a different workload principal")]
    SnapshotPrincipalMismatch,
    /// Runtime identity claims did not reference the supplied exact snapshot.
    #[error("runtime identity does not reference the supplied exact authorization snapshot")]
    SnapshotIdentityMismatch,
}

fn operation_set(
    operations: impl IntoIterator<Item = CapabilityOperation>,
) -> Result<BTreeSet<CapabilityOperation>, CapabilityError> {
    let mut normalized = BTreeSet::new();
    for operation in operations {
        if !normalized.insert(operation) {
            return Err(CapabilityError::DuplicateOperation(operation));
        }
        if normalized.len() > MAX_OPERATIONS_PER_CAPABILITY {
            return Err(CapabilityError::TooManyOperations);
        }
    }
    Ok(normalized)
}

fn validate_operations(
    resource_kind: CapabilityResourceKind,
    operations: &BTreeSet<CapabilityOperation>,
) -> Result<(), CapabilityError> {
    if let Some(operation) = operations
        .iter()
        .find(|operation| !operation.is_legal_for(resource_kind))
        .copied()
    {
        return Err(CapabilityError::IllegalOperation {
            operation,
            resource_kind,
        });
    }
    Ok(())
}

fn snapshot_hash(
    principal: WorkloadPrincipal,
    authorization_model_version: &str,
    bindings: &[CapabilityBinding],
) -> AuthorityHash {
    let mut hasher = CanonicalHasher::new(b"hephaestus.authorization-snapshot.v1");
    principal_canonicalize(principal, &mut hasher);
    hasher.text(authorization_model_version);
    hasher.usize(bindings.len());
    for binding in bindings {
        hasher.bytes(binding.normalized_hash().as_bytes());
    }
    hasher.finish()
}

fn principal_canonicalize(principal: WorkloadPrincipal, hasher: &mut CanonicalHasher) {
    hasher.text(principal.kind.as_str());
    hasher.uuid(principal.id);
    hasher.uuid(principal.revision_id);
}

struct CanonicalHasher(Sha256);

impl CanonicalHasher {
    fn new(domain: &[u8]) -> Self {
        let mut hasher = Self(Sha256::new());
        hasher.bytes(domain);
        hasher
    }

    fn bytes(&mut self, value: &[u8]) {
        self.0.update(value.len().to_be_bytes());
        self.0.update(value);
    }

    fn text(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn uuid(&mut self, value: Uuid) {
        self.bytes(value.as_bytes());
    }

    fn boolean(&mut self, value: bool) {
        self.bytes(&[u8::from(value)]);
    }

    fn usize(&mut self, value: usize) {
        self.bytes(&value.to_be_bytes());
    }

    fn operations(&mut self, operations: &BTreeSet<CapabilityOperation>) {
        let mut names = operations
            .iter()
            .map(|operation| operation.as_str())
            .collect::<Vec<_>>();
        names.sort_unstable();
        self.usize(names.len());
        for name in names {
            self.text(name);
        }
    }

    fn timestamp(&mut self, value: OffsetDateTime) {
        self.bytes(&value.unix_timestamp_nanos().to_be_bytes());
    }

    fn finish(self) -> AuthorityHash {
        AuthorityHash(self.0.finalize().into())
    }
}

#[cfg(test)]
mod tests {
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
        assert!(
            RuntimeSessionStatus::PendingHandoff.can_transition_to(RuntimeSessionStatus::Active)
        );
        assert!(RuntimeSessionStatus::Active.can_transition_to(RuntimeSessionStatus::Revoked));
        assert!(!RuntimeSessionStatus::Revoked.can_transition_to(RuntimeSessionStatus::Active));
        assert_eq!(
            RuntimeCredentialGeneration::new(0),
            Err(CapabilityError::InvalidCredentialGeneration)
        );
    }
}
