use run_domain::{Run, RunKind};
use run_orchestrator::{MailboxRuntimeEvent, RunRuntimeError, RunRuntimeInput};
use runtime_types::RunId;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;

use crate::{
    artifacts::{write_bytes, write_json},
    filesystem::runtime_error,
};

#[derive(Serialize)]
pub struct HostContext<'a> {
    schema_version: u8,
    run_id: RunId,
    run_kind: RunKind,
    instance_id: runtime_types::AgentInstanceId,
    instance_revision_id: runtime_types::AgentInstanceRevisionId,
    release_id: runtime_types::ReleaseId,
    release_agent_id: runtime_types::ReleaseAgentId,
    attachment_id: Option<runtime_types::AgentAttachmentId>,
    repository_id: Option<Uuid>,
    git_ref: Option<&'a str>,
    commit_sha: Option<&'a str>,
    release_mount: &'static str,
    repository_mount: &'static str,
    work_mount: &'static str,
    state_mount: Option<&'static str>,
    parameters_path: &'static str,
    update_id: Option<Uuid>,
    previous_revision_id: Option<Uuid>,
    previous_release_id: Option<Uuid>,
    previous_release_mount: Option<&'static str>,
    previous_parameters_path: Option<&'static str>,
    mailbox_event_path: Option<&'static str>,
    mailbox_body_path: Option<&'static str>,
}

impl<'a> HostContext<'a> {
    pub fn new(run: &Run, input: &'a RunRuntimeInput) -> Self {
        Self {
            schema_version: 1,
            run_id: run.id,
            run_kind: run.kind,
            instance_id: run.instance_id,
            instance_revision_id: run.instance_revision_id,
            release_id: run.release_id,
            release_agent_id: run.release_agent_id,
            attachment_id: run.attachment_id,
            repository_id: input.repository_id,
            git_ref: input.git_ref.as_deref(),
            commit_sha: input.commit_sha.as_deref(),
            release_mount: "/release",
            repository_mount: "/workspace/repo",
            work_mount: "/workspace/work",
            state_mount: run.requires_state.then_some("/var/lib/hephaestus"),
            parameters_path: "/run/hephaestus/parameters.json",
            update_id: input.update_id,
            previous_revision_id: input.previous_revision_id,
            previous_release_id: input.previous_release_id,
            previous_release_mount: input.previous_release_id.map(|_| "/release-previous"),
            previous_parameters_path: input
                .previous_parameters
                .as_ref()
                .map(|_| "/run/hephaestus/parameters-previous.json"),
            mailbox_event_path: input
                .mailbox_event
                .as_ref()
                .map(|_| "/run/hephaestus/mailbox-event.json"),
            mailbox_body_path: input
                .mailbox_event
                .as_ref()
                .map(|_| "/run/hephaestus/mailbox-body"),
        }
    }
}

#[derive(Serialize)]
struct MailboxControlEnvelope<'a> {
    schema_version: u8,
    mailbox_id: Uuid,
    event_id: Uuid,
    body_id: Uuid,
    method: &'a str,
    route: &'a str,
    selected_headers: &'a serde_json::Value,
    content_type: Option<&'a str>,
    trace_context: Option<&'a str>,
    received_at: time::OffsetDateTime,
    body_path: &'static str,
}

pub fn materialize_mailbox_event(
    control: &Path,
    event: &MailboxRuntimeEvent,
) -> Result<(), RunRuntimeError> {
    if event.body.len() > 1_048_576 {
        return Err(runtime_error("mailbox event body is invalid"));
    }
    if Sha256::digest(&event.body).as_slice() != event.integrity_hash {
        return Err(runtime_error("mailbox event body integrity is invalid"));
    }
    let envelope = MailboxControlEnvelope {
        schema_version: 1,
        mailbox_id: event.mailbox_id,
        event_id: event.event_id,
        body_id: event.body_id,
        method: &event.method,
        route: &event.route,
        selected_headers: &event.selected_headers,
        content_type: event.content_type.as_deref(),
        trace_context: event.trace_context.as_deref(),
        received_at: event.received_at,
        body_path: "/run/hephaestus/mailbox-body",
    };
    write_json(&control.join("mailbox-event.json"), &envelope)?;
    write_bytes(&control.join("mailbox-body"), &event.body)
}
