use super::{Fixture, SecretService};
use identity_domain::AuthenticatedIdentity;
use release_domain::{AgentInstanceId, AgentInstanceRevisionId};
use runtime_types::RunId;
use secret_application::RuntimeSecretAuthority;
use secret_domain::{SecretGrantId, SecretId, SecretImportId, SecretVersionId};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

/// Shared state passed between the lifecycle's independently sized phases.
pub struct LifecycleState {
    pub pool: PgPool,
    pub fixture: Fixture,
    pub service: Arc<SecretService<super::LocalKeyProvider>>,
    pub owner: AuthenticatedIdentity,
    pub target_manager: AuthenticatedIdentity,
    pub organization_secret_manager: AuthenticatedIdentity,
    pub ordinary_member: AuthenticatedIdentity,
    pub outsider: AuthenticatedIdentity,
    pub secret_id: SecretId,
    pub first_version: SecretVersionId,
    pub import_id: Option<SecretImportId>,
    pub repository_import_id: Option<SecretImportId>,
    pub repository_grant_id: Option<SecretGrantId>,
    pub instance_id: Option<AgentInstanceId>,
    pub initial_revision_id: Option<AgentInstanceRevisionId>,
    pub attachment_id: Option<Uuid>,
    pub second_attachment_id: Option<Uuid>,
    pub bound_revision_id: Option<AgentInstanceRevisionId>,
    pub repository_bound_revision_id: Option<AgentInstanceRevisionId>,
    pub run_id: Option<RunId>,
    pub brokered_binding_id: Option<Uuid>,
    pub brokered_rule_id: Option<Uuid>,
    pub authority: Option<RuntimeSecretAuthority>,
    pub raw_revision_id: Option<AgentInstanceRevisionId>,
    pub raw_run_id: Option<RunId>,
    pub raw_authority: Option<RuntimeSecretAuthority>,
    pub update_revision_id: Option<AgentInstanceRevisionId>,
    pub update_run_id: Option<RunId>,
    pub update_authority: Option<RuntimeSecretAuthority>,
    pub pinned_run: Option<RunId>,
    pub pinned_authority: Option<RuntimeSecretAuthority>,
    pub revoked_pinned_run: Option<RunId>,
    pub revoked_pinned_authority: Option<RuntimeSecretAuthority>,
    pub pre_revoked_run: Option<RunId>,
    pub pre_revoked_authority: Option<RuntimeSecretAuthority>,
    pub active_version: Option<Uuid>,
    pub expected_rotated_value: Option<String>,
}
