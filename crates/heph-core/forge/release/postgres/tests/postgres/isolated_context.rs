use super::*;

/// State after release publication and initial instance imports.
pub struct IsolatedPublishedContext {
    pub(super) pool: PgPool,
    pub(super) fixture: Fixture,
    pub(super) service: ReleaseService,
    pub(super) actor: AuthenticatedIdentity,
    pub(super) release_id: ReleaseId,
    pub(super) release_agent_id: ReleaseAgentId,
    pub(super) first_instance: AgentInstanceId,
    pub(super) first_revision: AgentInstanceRevisionId,
    pub(super) second_instance: AgentInstanceId,
}

/// State after immutable revision and update-gate setup.
pub struct IsolatedUpdateContext {
    pub(super) pool: PgPool,
    pub(super) fixture: Fixture,
    pub(super) service: ReleaseService,
    pub(super) actor: AuthenticatedIdentity,
    pub(super) release_id: ReleaseId,
    pub(super) first_instance: AgentInstanceId,
    pub(super) second_instance: AgentInstanceId,
    pub(super) revised_first: AgentInstanceRevisionId,
    pub(super) update_release_agent: ReleaseAgentId,
    pub(super) update_release_id: Uuid,
    pub(super) deferred_attachment: AgentAttachmentId,
    pub(super) deferred_receive: Uuid,
    pub(super) prior_request_id: Uuid,
    pub(super) deferred_commit: String,
    pub(super) update_id: AgentUpdateId,
    pub(super) update_candidate_revision: AgentInstanceRevisionId,
}

/// State after mailbox and transport gate setup.
pub struct IsolatedGateContext {
    pub(super) pool: PgPool,
    pub(super) fixture: Fixture,
    pub(super) service: ReleaseService,
    pub(super) actor: AuthenticatedIdentity,
    pub(super) release_id: ReleaseId,
    pub(super) first_instance: AgentInstanceId,
    pub(super) second_instance: AgentInstanceId,
    pub(super) revised_first: AgentInstanceRevisionId,
    pub(super) update_release_agent: ReleaseAgentId,
    pub(super) update_release_id: Uuid,
    pub(super) deferred_attachment: AgentAttachmentId,
    pub(super) deferred_receive: Uuid,
    pub(super) prior_request_id: Uuid,
    pub(super) deferred_commit: String,
    pub(super) update_id: AgentUpdateId,
    pub(super) update_candidate_revision: AgentInstanceRevisionId,
    pub(super) mailbox_store: PostgresMailboxRepository,
    pub(super) mailbox_id: MailboxId,
    pub(super) accepted: mailbox_postgres::AcceptedMailboxEvent,
    pub(super) dispatch: MailboxDispatchCommand,
    pub(super) nats_event_id: Option<MailboxEventId>,
}
