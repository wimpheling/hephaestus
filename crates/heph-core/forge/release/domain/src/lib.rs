//! Provider-neutral reusable-agent release, instance, attachment, and update
//! contracts.
//!
//! A product-level [`AgentInstance`] is a durable project-owned aggregate. It
//! is deliberately unrelated to `vm_trait::VmInstance`, which is one
//! ephemeral provider allocation used while executing a run.

pub use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, ReleaseId,
};

pub mod ui;
pub mod ui_browser;
pub mod ui_installation;

pub use ui_installation::{
    UiInstallationCallerKey, UiInstallationCommandIdentity, UiInstallationInputDigest,
    UiInstallationOperation, UiInstallationState, UiInstallationTarget,
};

mod errors;
pub use errors::ReleaseValueError;

mod identifiers;
pub use identifiers::{
    AgentFamilyId, AgentUpdateId, BuildRequestId, DeferredTriggerId, ReleaseArtifactId,
    UiInstallationGenerationId, UiInstallationId,
};

mod value_objects;
pub use value_objects::{
    AgentKey, ArtifactPath, InstanceName, ParameterName, RefSelector, ReleaseVersion,
};

mod lifecycle;
pub use lifecycle::{BuildState, InstanceState, ReleaseState, UpdateState};

mod hashing;
pub use hashing::{ContentHash, ReleaseCommandKey};

mod release_models;
pub use release_models::{
    AgentFamily, ArtifactKind, BuildRequest, NetworkAccess, Release, ReleaseAgent, ReleaseArtifact,
    RuntimePolicy, UpdateHook,
};

mod parameters;
pub use parameters::{
    ParameterDeclaration, ParameterDiagnostic, ParameterDiagnosticCode, ParameterDocument,
    ParameterType, ParameterValue,
};

mod instances;
pub use instances::{
    AgentAttachment, AgentInstance, AgentInstanceRevision, AgentUpdate, DeferredTrigger,
    ExactRunProvenance, RevisionDiagnostic, TriggerPolicy,
};

mod validation;
pub use validation::{validate_attachment_project, validate_update_family};

#[cfg(test)]
mod tests;
