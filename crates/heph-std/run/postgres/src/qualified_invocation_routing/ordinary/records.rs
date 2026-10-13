//! Narrow actual row projections and exact request/Start comparison.

use super::{QualifiedOrdinaryGitSource, QualifiedOrdinaryRoutingHoldReason as Hold};
use crate::{errors::storage, model::parse_run_kind};
use run_domain::StartRun;
use run_orchestrator::RepositoryError;
use serde_json::Value;
use uuid::Uuid;

// Independent database flags retain the SQLx row projection shape.
#[allow(clippy::struct_excessive_bools)]
#[derive(sqlx::FromRow)]
pub(super) struct OriginalRun {
    pub(super) id: Uuid,
    command_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Option<Uuid>,
    run_kind: String,
    requires_state: bool,
    pub(super) execution_profile: String,
    pub(super) legacy_vm_placement_required: bool,
    pub(super) vm_id: Option<String>,
    pub(super) vm_provider_namespace: Option<String>,
    pub(super) vm_host_id: Option<String>,
    pub(super) vm_binding_kind: Option<String>,
    pub(super) vm_plan_admission_required: bool,
    pub(super) state: String,
    pub(super) cancellation_requested: bool,
    pub(super) acquisition_closed: bool,
}

impl OriginalRun {
    pub(super) fn is_invocation(&self) -> bool {
        self.run_kind == "invocation"
    }
    pub(super) fn start(&self) -> Result<StartRun, RepositoryError> {
        Ok(StartRun {
            command_id: self.command_id.into(),
            run_id: self.id.into(),
            instance_id: self.instance_id.into(),
            instance_revision_id: self.instance_revision_id.into(),
            release_id: self.release_id.into(),
            release_agent_id: self.release_agent_id.into(),
            attachment_id: self.attachment_id.map(Into::into),
            kind: parse_run_kind(&self.run_kind)?,
            requires_state: self.requires_state,
        })
    }
}

// SQL column names distinguish producer kind from original request provenance.
#[allow(clippy::struct_field_names)]
#[derive(sqlx::FromRow)]
pub(super) struct Request {
    pub(super) id: Uuid,
    run_id: Uuid,
    command_id: Uuid,
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Uuid,
    requires_state: bool,
    request_kind: String,
    repository_id: Uuid,
    commit_sha: String,
    git_ref: String,
    receive_id: Option<Uuid>,
    trigger_command_id: Option<Uuid>,
    pub(super) actor_id: Option<Uuid>,
    pub(super) request_id: Option<Uuid>,
    retry_of_run_id: Option<Uuid>,
    attempt: i32,
    platform_policy_version: String,
}

impl Request {
    pub(super) fn compare(&self, supplied: Option<&StartRun>) -> Result<StartRun, Hold> {
        if self.request_kind != "instance_normal" {
            return Err(Hold::UnsupportedProducer);
        }
        if [
            self.id,
            self.run_id,
            self.command_id,
            self.instance_id,
            self.instance_revision_id,
            self.release_id,
            self.release_agent_id,
            self.attachment_id,
            self.repository_id,
        ]
        .iter()
        .any(Uuid::is_nil)
            || [
                self.receive_id,
                self.trigger_command_id,
                self.actor_id,
                self.request_id,
                self.retry_of_run_id,
            ]
            .into_iter()
            .flatten()
            .any(|id| id.is_nil())
            || self.receive_id.is_some() == self.trigger_command_id.is_some()
            || self.attempt < 1
            || (self.attempt == 1) != self.retry_of_run_id.is_none()
            || !matches!(self.commit_sha.len(), 40 | 64)
            || !self
                .commit_sha
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !self.git_ref.starts_with("refs/")
            || self.git_ref.len() > 4096
            || self.git_ref.chars().any(char::is_control)
            || self.platform_policy_version.is_empty()
            || self.platform_policy_version.len() > 128
            || self.platform_policy_version.trim() != self.platform_policy_version
            || self.platform_policy_version.chars().any(char::is_control)
        {
            return Err(Hold::ContradictoryOriginal);
        }
        let start = StartRun {
            command_id: self.command_id.into(),
            run_id: self.run_id.into(),
            instance_id: self.instance_id.into(),
            instance_revision_id: self.instance_revision_id.into(),
            release_id: self.release_id.into(),
            release_agent_id: self.release_agent_id.into(),
            attachment_id: Some(self.attachment_id.into()),
            kind: run_domain::RunKind::Normal,
            requires_state: self.requires_state,
        };
        if supplied.is_some_and(|command| command != &start) {
            return Err(Hold::ContradictoryOriginal);
        }
        Ok(start)
    }
    pub(super) fn source(&self) -> QualifiedOrdinaryGitSource {
        QualifiedOrdinaryGitSource {
            producer_request_id: self.id,
            repository_id: self.repository_id,
            commit_sha: self.commit_sha.clone(),
            git_ref: self.git_ref.clone(),
            receive_id: self.receive_id,
            trigger_command_id: self.trigger_command_id,
            original_actor: self.actor_id,
            original_request: self.request_id,
            retry_of_run_id: self.retry_of_run_id,
            attempt: self.attempt,
            platform_policy_version: self.platform_policy_version.clone(),
        }
    }
}

#[derive(sqlx::FromRow)]
pub(super) struct Inbox {
    command_type: String,
    payload: Value,
    pub(super) processed: bool,
}
impl Inbox {
    pub(super) fn matches(&self, start: &StartRun) -> Result<bool, RepositoryError> {
        Ok(self.command_type == "start_run"
            && self.payload == serde_json::to_value(start).map_err(storage)?)
    }
}
