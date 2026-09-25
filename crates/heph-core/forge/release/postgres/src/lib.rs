//! `PostgreSQL` release and instance command adapter.

use agent_config::{AgentConfig, NetworkProfile, ParameterDefault, REUSABLE_RELEASE_VERSION};
use authz_domain::{AuthorizationDecision, ObjectRef, ObjectType, Permission, Subject};
use authz_postgres::{PostgresMelangeAuthorizer, audit_decision, begin_actor_transaction};
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
};
use capability_domain::{
    CapabilityBinding, CapabilityBindingId, CapabilityOperation, CapabilityRequirement,
    CapabilityRequirementId, CapabilityResource, CapabilityResourceKind, CapabilitySlotKey,
};
use git_capability_domain::{
    BoundGitCapability, BranchRefPolicy, BranchUpdatePolicy, ChangedPathGlob, GitCapabilityCeiling,
    GitCapabilityCeilingInput, GitOperation, RefGlob, RefMutationPermission, RefNamespacePolicy,
    RefUpdatePolicy, RepositoryId as GitRepositoryId, TransferLimits,
};
use identity_domain::AuthenticatedIdentity;
use release_artifact_helpers::{
    artifact_kind_name, artifact_manifest_hash, parameter_schema, policy_from_contract,
    runtime_policy, validate_artifact,
};
use release_brokered_helpers::{
    candidate_accepts_binding, clone_brokered_secret_rule, required_slot_diagnostics,
    update_contract_diagnostics,
};
use release_capability_binding::{
    capability_diagnostics_from_json, insert_capability_binding, insert_git_capability_binding,
    insert_release_git_ceiling, operation_names, release_capability_requirements,
    release_capability_requirements_hash,
};
use release_capability_helpers::{
    git_authority_matches_capability, load_capability_requirements,
    load_carried_capability_bindings, load_git_ceilings, stored_requirement,
};
use release_capability_selection::{
    authorize_capability_selection, capability_resource_is_in_project,
};
use release_command_events::{
    append_event, append_instance_event, decode_hash, existing_command,
    is_update_admission_generation_conflict, record_command, ref_selector_string,
    trigger_policy_name,
};
use release_domain::{
    AgentAttachmentId, AgentFamilyId, AgentInstanceId, AgentInstanceRevisionId, AgentKey,
    AgentUpdateId, ArtifactKind, ArtifactPath, ContentHash, NetworkAccess, ParameterDeclaration,
    ParameterDocument, ParameterName, ParameterType, ParameterValue, RefSelector, ReleaseAgentId,
    ReleaseCommandKey, ReleaseId, RuntimePolicy,
};
pub use release_errors::ReleaseServiceError;
use release_revision_persistence::{
    clone_revision_binding, insert_revision, insert_revision_with_binding_ids,
    insert_update_candidate_revision,
};
use release_rows::{
    BrokeredRuleCloneRow, CapabilityRequirementRow, CapabilityRevisionSourceRow,
    CarriedCapabilityBinding, DeferredMaterializationRow, GitCapabilityRow, ReleaseAgentRow,
    RevisionBindingRow, RevisionUpdateRow, StoredCapabilityBindingRow, UpdateCandidateRow,
    UpdateCurrentRow, UpdateHookAdmissionRow, UpdateLifecycleRow, UpdateRecoveryRow,
    UpdateRunResultRow, append_rejected_update_event, append_uncertain_update_events,
};
pub use release_service::*;
use release_update_gate::{
    enqueue_mailbox_wakes, existing_terminal_decision, mark_volume_lease,
    materialize_deferred_triggers, recovery_decision, reopen_after_update,
};
use run_domain::{RunKind, StartRun};
use runtime_types::{CommandId, RunId};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Transaction};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use time::OffsetDateTime;
pub use ui_browser::PgUiBrowserSessionStore;
pub use ui_browser_resources::{PgUiBrowserServingStore, PgUiGenerationHostResolver};
pub use ui_installation_navigation::PgUiInstallationNavigator;
pub use ui_request_audit::{
    PgUiRequestAuditRepository, append_in_transaction as append_ui_request_audit_in_transaction,
};
use uuid::Uuid;

mod release_artifact_helpers;
mod release_attachments;
mod release_authorization;
mod release_brokered_helpers;
mod release_build;
mod release_capability_binding;
mod release_capability_helpers;
mod release_capability_revision;
mod release_capability_selection;
mod release_command_events;
mod release_errors;
mod release_instance_import;
mod release_instance_revision;
mod release_publication;
mod release_revision_persistence;
mod release_rows;
mod release_service_access;
mod release_update_admission;
mod release_update_creation;
mod release_update_finalization;
mod release_update_gate;
mod release_update_preparation;
mod release_update_recovery;
mod ui_browser;
mod ui_browser_resources;
mod ui_installation;
mod ui_installation_navigation;
mod ui_publication;
mod ui_request_audit;

const RUN_START_SUBJECT: &str = "hephaestus.run.start";
const MAILBOX_WAKE_SUBJECT: &str = "heph.mailbox.v1.wake";
const MAILBOX_WAKE_EVENT_TYPE: &str = "mailbox.wake.v1";

/// PostgreSQL-backed release and instance command service.
pub struct ReleaseService {
    pool: PgPool,
    authorizer: Arc<PostgresMelangeAuthorizer>,
}

impl ReleaseService {}

#[cfg(test)]
mod release_helper_tests;

#[cfg(test)]
mod ui_schema_tests;
