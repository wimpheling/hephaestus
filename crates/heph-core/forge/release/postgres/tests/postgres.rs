//! Opt-in real-PostgreSQL reusable release and isolated instance coverage.

use agent_config::{AgentConfig, parse, parse_repository_gateways, parse_repository_uis};
use authz_postgres::PostgresMelangeAuthorizer;
use capability_domain::{
    CapabilityBindingId, CapabilityOperation, CapabilityResource, CapabilityResourceKind,
    CapabilitySlotKey,
};
use forge_domain::{GitRef, ProjectId, RepositoryId};
use futures_util::StreamExt;
use identity_domain::{
    AuthenticatedIdentity, OrganizationId, RequestId, UserId, actor_idempotency_id,
};
use mailbox_dispatch::{
    MAILBOX_DISPATCH_SUBJECT, MAILBOX_WAKE_SUBJECT, MailboxDispatchCommand, MailboxDispatchStore,
};
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, MailboxEvent, MailboxEventId, MailboxId,
    MailboxOperationIdentity, ProducerId,
};
use mailbox_postgres::PostgresMailboxRepository;
use release_domain::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, AgentUpdateId, ArtifactKind,
    ArtifactPath, BuildRequestId, ContentHash, InstanceName, NetworkAccess, ParameterName,
    ParameterValue, RefSelector, ReleaseAgentId, ReleaseArtifactId, ReleaseCommandKey, ReleaseId,
    ReleaseVersion, RuntimePolicy, TriggerPolicy, UiInstallationCallerKey,
    UiInstallationCommandIdentity, UiInstallationGenerationId, UiInstallationOperation,
    UiInstallationTarget,
};
use release_postgres::{
    ActivateUiInstallation, BeginUpdateHook, CompleteBuild, CreateAttachment, CreateInstanceUpdate,
    DisableUiInstallation, ImportAgent, InstallStaticUi, InstallStaticUiResult, InstallUi,
    RecoverInstanceUpdate, ReleaseArtifactInput, ReleaseService, RemoveAttachment,
    RemoveUiInstallation, ReviseInstance, ReviseInstanceCapabilities, RollbackUiInstallation,
    SetAttachmentEnabled, UiInstallationError, UiInstallationGenerationResult, UpdateDecision,
    UpdateHookResult, UpdateRecoveryAction, UpdateRecoveryDecision,
};
use runtime_types::RunId;
use serde_json::{Value, json};
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use time::OffsetDateTime;
use uuid::Uuid;

type UiBindingRow = (String, String, Uuid, Uuid, Uuid, String, String, String);

struct Fixture {
    actor: UserId,
    organization: OrganizationId,
    first_project: ProjectId,
    first_repository: RepositoryId,
    first_aux_repository: RepositoryId,
    second_project: ProjectId,
    second_repository: RepositoryId,
    build: BuildRequestId,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct BrokerRuleSnapshot {
    id: Uuid,
    binding_id: Uuid,
    instance_revision_id: Uuid,
    secret_version_id: Uuid,
    destination_origin: String,
    location_kind: String,
    header_name: String,
    header_prefix: Option<String>,
    normalized_hash: Vec<u8>,
}

#[path = "postgres/mod.rs"]
pub mod support;
use support::*;

#[path = "postgres/broker_rules.rs"]
pub mod broker_rules;

#[path = "postgres/complete_build.rs"]
pub mod complete_build;

#[path = "postgres/installation.rs"]
pub mod installation;

#[path = "postgres/install_replay.rs"]
mod install_replay;

#[path = "postgres/parent_move.rs"]
mod parent_move;

#[path = "postgres/runtime_git.rs"]
mod runtime_git;

/// Shared state passed between the isolated-instance test phases.
#[path = "postgres/isolated_context.rs"]
pub mod isolated_context;
use isolated_context::{
    IsolatedAttachmentContext, IsolatedGateContext, IsolatedPublishedContext,
    IsolatedRecoveryContext, IsolatedTransportContext, IsolatedUpdateContext,
};

/// Release publication and initial instance setup phase.
#[path = "postgres/isolated_setup.rs"]
pub mod isolated_setup;

/// Immutable revision and update-gate setup phase.
#[path = "postgres/isolated_updates.rs"]
pub mod isolated_updates;

/// Mailbox and transport gate setup phase.
#[path = "postgres/isolated_mailbox.rs"]
pub mod isolated_mailbox;

/// Local gate transition and deferred trigger phase.
#[path = "postgres/isolated_gate_activation.rs"]
pub mod isolated_gate_activation;

/// Post-activation `PostgreSQL` and optional NATS replay phase.
#[path = "postgres/isolated_nats_replay.rs"]
pub mod isolated_nats_replay;

/// Agent rejection and uncertain update recovery phase.
#[path = "postgres/isolated_rejection_recovery.rs"]
pub mod isolated_rejection_recovery;

/// Activation CAS recovery phase.
#[path = "postgres/isolated_activation_recovery.rs"]
pub mod isolated_activation_recovery;

/// Attachment isolation and final durable history phase.
#[path = "postgres/isolated_attachments.rs"]
pub mod isolated_attachments;

/// `JetStream` outbox retry and deduplication scenario.
#[path = "postgres/outbox_retry.rs"]
pub mod outbox_retry;

#[tokio::test]
#[serial]
// The real JetStream pull streams are retained across the gate transition so
// this single integration test can prove transport redelivery and claim CAS.
#[allow(clippy::large_stack_frames)]
async fn publishes_once_and_imports_isolated_instances_with_exact_attachments() {
    let Some(context) = isolated_setup::prepare().await else {
        return;
    };
    let context = isolated_updates::prepare(context).await;
    let context = isolated_gate_activation::prepare(isolated_mailbox::prepare(context).await).await;
    let context = isolated_nats_replay::prepare(context).await;
    let context = isolated_rejection_recovery::prepare(context).await;
    let context = isolated_activation_recovery::prepare(context).await;
    isolated_attachments::verify(context).await;
}
