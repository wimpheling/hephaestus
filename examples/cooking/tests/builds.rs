//! Helpers for exercising the cooking build and release path through the app.
//!
//! The golden fixture passes its database, daemon, and identity context here
//! so these operations exercise the production RPC boundaries.

use forge_domain::{GitRef, ProjectId, RepositoryId};
use forge_postgres::PgForgeRepository;
use forge_service::CreateRepository;
use hephaestus_app::RunningHephaestus;
use identity_domain::{AuthenticatedIdentity, UserId};
use rpc_proto::{
    connect::hephaestus::{
        build::v1::BuildServiceClient, gateway::v1::GatewayServiceClient,
        instance::v1::AgentInstanceServiceClient, release::v1::ReleaseServiceClient,
    },
    messages::hephaestus::{
        build::v1::{BuildState, GetBuildRequest},
        common::v1::{NetworkPolicy, OpaqueId, ParameterValue, RequestContext, RuntimePolicy},
        gateway::v1::{
            ConfigureGatewayRequest, CreateMailboxBindingRequest, GatewaySecretSelection,
            GetGatewayRequest, InstallReleaseGatewaysRequest, ListProjectGatewaysRequest,
        },
        instance::v1::{
            CreateAttachmentRequest, CreateMailboxRequest, ImportAgentRequest, RefSelector,
            TriggerPolicy, ref_selector,
        },
        release::v1::{GetReleaseRequest, PublishReleaseRequest, SetDraftVersionRequest},
    },
};
use serde_json::Value;
use sqlx::PgPool;
use std::{
    error::Error,
    fs, io,
    os::unix::fs as unix_fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::{process::Command, time::sleep};
use uuid::Uuid;

/// Error returned when source preparation, build observation, or publication
/// crosses a production boundary unsuccessfully.
pub type BuildError = Box<dyn Error + Send + Sync>;

/// Authentication material needed by the Git and Connect boundaries.
///
/// Git uses the OIDC bearer assertion accepted by the smart HTTP endpoint;
/// the RPC token factory creates a short-lived method-audience-bound mediator
/// assertion for each call.  The factory is required because the mediator
/// rejects assertions whose audience is a different RPC procedure.
#[derive(Clone, Copy)]
pub struct CookingIdentity<'a> {
    /// Actor used when creating the repositories and persisted build rows.
    pub actor: &'a AuthenticatedIdentity,
    /// Bearer token accepted by Git smart HTTP.
    pub git_token: &'a str,
    /// Creates a fresh bearer token for the exact Connect procedure path.
    pub rpc_token: &'a (dyn Fn(&str) -> String + Send + Sync),
}

/// Existing fixture resources required by [`build_and_publish`].
pub struct CookingBuildContext<'a> {
    /// Application database used only to observe receive/build state.
    pub pool: &'a PgPool,
    /// Running daemon exposing Git HTTP and Connect RPC.
    pub running: &'a RunningHephaestus,
    /// Per-test root under which temporary source checkouts are created.
    pub root: &'a Path,
    /// Selected canonical source root (the same override used by the runner).
    pub source_root: &'a Path,
    /// Project receiving the two cooking source repositories.
    pub project_id: ProjectId,
    /// Trusted repository factory from the already-configured test fixture.
    pub repositories: &'a PgForgeRepository,
    /// Authenticated actor and boundary tokens.
    pub identity: CookingIdentity<'a>,
    /// Bound for each asynchronous build/release wait.
    pub timeout: Duration,
}

/// Waits until all asynchronous build work belonging to the cooking project
/// has reached a durable terminal state before an intentional daemon restart.
///
/// The OCI materialization queue has no project column, so it is narrowed by
/// the exact cooking worker name and an image reference owned by this project.
/// A failed terminal row is reported immediately; silently restarting over a
/// failed build would make the later cooking assertions misleading.
pub async fn wait_for_cooking_build_quiescence(
    pool: &PgPool,
    project_id: ProjectId,
    materialization_worker_name: &str,
    timeout: Duration,
) {
    assert!(!materialization_worker_name.trim().is_empty());
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let counts =
            cooking_build_queue_counts(pool, project_id, materialization_worker_name).await;
        if counts.build_failed != 0 {
            let details = cooking_build_failure_details(pool, project_id).await;
            assert_eq!(
                counts.build_failed, 0,
                "cooking build queue contains failed or cancelled terminal work:\n{details}"
            );
        }
        assert_eq!(
            counts.production_failed, 0,
            "cooking OCI production queue contains failed terminal work"
        );
        assert_eq!(
            counts.definition_failed, 0,
            "cooking OCI definition contains failed terminal work"
        );
        assert_eq!(
            counts.materialization_failed, 0,
            "cooking OCI materialization queue contains failed terminal work"
        );
        if counts.is_quiescent() {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "cooking build queues did not quiesce: build_pending={}, \
             production_pending={}, definition_pending={}, materialization_pending={}",
            counts.build_pending,
            counts.production_pending,
            counts.definition_pending,
            counts.materialization_pending
        );
        sleep(Duration::from_millis(250)).await;
    }
}

/// Returns bounded, credential-redacted details for failed build rows before
/// the queue assertion aborts the scenario. The normal Build RPC exposes the
/// same fields; this project-wide queue observer uses the fixture pool to
/// retain guest stderr before teardown removes the disposable database.
async fn cooking_build_failure_details(pool: &PgPool, project_id: ProjectId) -> String {
    let rows: Vec<CookingBuildFailure> = sqlx::query_as(
        "SELECT build.id, build.source_commit, build.state,
                execution.failure_code, execution.exit_code,
                execution.exit_signal, execution.logs
           FROM build_requests build
           JOIN repositories repository ON repository.id = build.repository_id
           LEFT JOIN build_executions execution
             ON execution.build_request_id = build.id
          WHERE repository.project_id = $1
            AND build.state IN ('failed', 'cancelled')
          ORDER BY build.created_at, build.id
          LIMIT 16",
    )
    .bind(project_id.as_uuid())
    .fetch_all(pool)
    .await
    .expect("inspect failed cooking build details");

    rows.into_iter()
        .map(|failure| {
            let logs = failure
                .logs
                .and_then(|value| value.as_array().cloned())
                .map(|entries| {
                    entries
                        .into_iter()
                        .filter_map(|entry| {
                            let stream = entry.get("stream")?.as_str()?;
                            let text = entry.get("text")?.as_str()?;
                            Some(format!("[{stream}] {}", redact_build_log(text)))
                        })
                        .collect::<Vec<_>>()
                        .join("\\n")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| String::from("<no retained guest logs>"));
            format!(
                "build_id={} state={} source_commit={} failure_code={:?} \
                     exit_code={:?} exit_signal={:?} logs={}",
                failure.id,
                failure.state,
                failure.source_commit,
                failure.failure_code,
                failure.exit_code,
                failure.exit_signal,
                logs
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(sqlx::FromRow)]
struct CookingBuildFailure {
    id: Uuid,
    source_commit: String,
    state: String,
    failure_code: Option<String>,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
    logs: Option<Value>,
}

#[derive(Debug)]
struct CookingBuildQueueCounts {
    build_pending: i64,
    build_failed: i64,
    production_pending: i64,
    production_failed: i64,
    definition_pending: i64,
    definition_failed: i64,
    materialization_pending: i64,
    materialization_failed: i64,
}

impl CookingBuildQueueCounts {
    const fn is_quiescent(&self) -> bool {
        self.build_pending == 0
            && self.production_pending == 0
            && self.definition_pending == 0
            && self.materialization_pending == 0
    }
}

async fn cooking_build_queue_counts(
    pool: &PgPool,
    project_id: ProjectId,
    materialization_worker_name: &str,
) -> CookingBuildQueueCounts {
    let counts: (i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
                (SELECT count(DISTINCT build.id)
                   FROM build_requests build
                   JOIN repositories repository ON repository.id = build.repository_id
                  WHERE repository.project_id = $1
                    AND build.state IN ('queued', 'running', 'importing')),
                (SELECT count(DISTINCT build.id)
                   FROM build_requests build
                   JOIN repositories repository ON repository.id = build.repository_id
                  WHERE repository.project_id = $1
                    AND build.state IN ('failed', 'cancelled')),
                (SELECT count(DISTINCT production.id)
                   FROM repository_oci_image_production_jobs production
                   JOIN repository_oci_image_definitions definition
                     ON definition.id = production.definition_id
                  WHERE definition.project_id = $1
                    AND production.state IN ('queued', 'claimed')),
                (SELECT count(DISTINCT production.id)
                   FROM repository_oci_image_production_jobs production
                   JOIN repository_oci_image_definitions definition
                     ON definition.id = production.definition_id
                  WHERE definition.project_id = $1
                    AND production.state = 'failed'),
                (SELECT count(DISTINCT definition.id)
                   FROM repository_oci_image_definitions definition
                  WHERE definition.project_id = $1
                    AND definition.status = 'producing'),
                (SELECT count(DISTINCT definition.id)
                   FROM repository_oci_image_definitions definition
                  WHERE definition.project_id = $1
                    AND definition.status = 'failed'),
                (SELECT count(DISTINCT materialization.id)
                   FROM oci_image_materialization_jobs materialization
                  WHERE materialization.worker_name = $2
                    AND materialization.state IN ('queued', 'claimed')
                    AND EXISTS (
                        SELECT 1
                          FROM repository_oci_image_definitions definition
                         WHERE definition.project_id = $1
                           AND definition.image_reference = materialization.image_reference
                    )),
                (SELECT count(DISTINCT materialization.id)
                   FROM oci_image_materialization_jobs materialization
                  WHERE materialization.worker_name = $2
                    AND materialization.state = 'failed'
                    AND EXISTS (
                        SELECT 1
                          FROM repository_oci_image_definitions definition
                         WHERE definition.project_id = $1
                           AND definition.image_reference = materialization.image_reference
                    ))",
    )
    .bind(project_id.as_uuid())
    .bind(materialization_worker_name)
    .fetch_one(pool)
    .await
    .expect("inspect cooking build queues");
    CookingBuildQueueCounts {
        build_pending: counts.0,
        build_failed: counts.1,
        production_pending: counts.2,
        production_failed: counts.3,
        definition_pending: counts.4,
        definition_failed: counts.5,
        materialization_pending: counts.6,
        materialization_failed: counts.7,
    }
}

/// Provenance and IDs returned for one published cooking source repository.
#[derive(Debug, Clone)]
pub struct PublishedCookingRepository {
    /// Repository metadata ID.
    pub repository_id: RepositoryId,
    /// Authenticated actor which owns the created repository metadata.
    pub actor_id: UserId,
    /// Exact commit pushed through Git HTTP.
    pub source_commit: String,
    /// Build request created by the accepted push.
    pub build_request_id: Uuid,
    /// Release generated from the successful isolated build.
    pub release_id: Uuid,
    /// Agent identity inside the published release used by instance updates.
    pub release_agent_id: Uuid,
    /// Version assigned through `SetDraftVersion`.
    pub version: String,
    /// Canonical hash returned by the Build service.
    pub build_definition_hash: String,
    /// Normalized configuration hash returned by the Build service.
    pub configuration_hash: String,
    /// Release manifest hash returned by the Release service.
    pub manifest_hash: String,
    /// Temporary checkout retained under the fixture root until test cleanup.
    pub source_path: PathBuf,
    /// Mutable Git checkout used when publishing successive same-family
    /// update commits.  Callers should use [`Self::source_path`] for
    /// provenance because this path changes for each variant.
    pub working_path: PathBuf,
}

/// Results for both canonical cooking repositories.
#[derive(Debug, Clone)]
pub struct PublishedCookingBuilds {
    /// Rust gateway source and release.
    pub gateway: PublishedCookingRepository,
    /// Python agent source and release.
    pub agent: PublishedCookingRepository,
}

/// IDs returned by the production instance commands for a published cooking
/// agent and its separate blog repository attachment.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy)]
pub struct PreparedCookingInstance {
    /// Instance created by `ImportAgent`.
    pub instance_id: Uuid,
    /// Immutable revision created by `ImportAgent`.
    pub revision_id: Uuid,
    /// Push attachment created against the blog repository.
    pub attachment_id: Uuid,
    /// Mailbox created by the instance service for the cooking run.
    pub mailbox_id: Uuid,
}

/// Separate Git repository attached to the cooking instance and its exact
/// pushed input commit.
#[derive(Debug, Clone)]
pub struct PreparedCookingBlog {
    /// Repository metadata ID.
    pub repository_id: RepositoryId,
    /// Exact commit accepted by the production Git receive path.
    pub source_commit: String,
}

/// Active gateway identifiers returned after installing its released
/// declaration and reading it back through the query API.
#[derive(Debug, Clone, Copy)]
pub struct InstalledCookingGateway {
    /// Gateway metadata identifier.
    pub gateway_id: Uuid,
    /// Active immutable declaration revision.
    pub revision_id: Uuid,
}

/// Result of configuring the gateway and creating its ordinary mailbox grant.
#[derive(Debug, Clone, Copy)]
pub struct ConfiguredCookingGateway {
    /// New immutable configured gateway revision.
    pub revision_id: Uuid,
    /// Revocable grant created by `CreateMailboxBinding`.
    pub grant_id: Uuid,
}

/// Slot used only by the joined authority probe. It is deliberately absent
/// from the released gateway declaration while a real second mailbox exists.
pub const FOREIGN_PUBLICATION_SLOT: &str = "cooking_foreign_requests";

/// Returns the cooking agent parameter payload used by the production import
/// and update RPC helpers.
pub fn cooking_agent_parameters() -> Vec<ParameterValue> {
    cooking_agent_parameters_for_rules(super::cooking::MODEL_RULE, super::cooking::RELAY_RULE)
}

/// Returns typed parameters for an instance whose immutable broker rules were
/// allocated before import. This keeps transformed guest releases on the
/// ordinary ImportAgent/build path while allowing a separate rule namespace.
pub fn cooking_agent_parameters_for_rules(
    model_rule_id: uuid::Uuid,
    relay_rule_id: uuid::Uuid,
) -> Vec<ParameterValue> {
    vec![
        ParameterValue {
            name: String::from("model_rule_id"),
            value: Some(
                rpc_proto::messages::hephaestus::common::v1::parameter_value::Value::StringValue(
                    model_rule_id.to_string(),
                ),
            ),
            ..Default::default()
        },
        ParameterValue {
            name: String::from("relay_rule_id"),
            value: Some(
                rpc_proto::messages::hephaestus::common::v1::parameter_value::Value::StringValue(
                    relay_rule_id.to_string(),
                ),
            ),
            ..Default::default()
        },
    ]
}

/// Returns the host-to-guest inbound placeholder for the selected secret
/// version. The gateway edge rewrites the inbound credential using this
/// version UUID before the released guest receives the request.
pub fn cooking_inbound_placeholder(secret_version_id: Uuid) -> String {
    format!("heph-placeholder:v1:{secret_version_id}")
}

/// Returns the exact typed gateway parameter payload for a cooking fixture.
pub fn cooking_gateway_parameters(
    inbound_placeholder: &str,
    alice_provider_id: i64,
    bob_provider_id: i64,
) -> Vec<ParameterValue> {
    vec![
        ParameterValue {
            name: String::from("inbound_placeholder"),
            value: Some(
                rpc_proto::messages::hephaestus::common::v1::parameter_value::Value::StringValue(
                    inbound_placeholder.to_owned(),
                ),
            ),
            ..Default::default()
        },
        ParameterValue {
            name: String::from("alice_provider_id"),
            value: Some(
                rpc_proto::messages::hephaestus::common::v1::parameter_value::Value::IntegerValue(
                    alice_provider_id,
                ),
            ),
            ..Default::default()
        },
        ParameterValue {
            name: String::from("bob_provider_id"),
            value: Some(
                rpc_proto::messages::hephaestus::common::v1::parameter_value::Value::IntegerValue(
                    bob_provider_id,
                ),
            ),
            ..Default::default()
        },
    ]
}

/// Imports the published cooking agent and attaches a separately created blog
/// repository through the production instance RPCs.
pub async fn prepare_cooking_instance(
    context: &CookingBuildContext<'_>,
    release_agent_id: Uuid,
    blog_repository_id: RepositoryId,
    parameters: Vec<ParameterValue>,
) -> Result<PreparedCookingInstance, BuildError> {
    prepare_cooking_instance_variant(
        context,
        release_agent_id,
        blog_repository_id,
        parameters,
        "cooking-agent",
        "cooking-agent",
    )
    .await
}

/// Creates an additional instance with distinct durable command identities
/// and a distinct bounded name, for joined authority probes that need a real
/// foreign mailbox in the same project.
pub async fn prepare_cooking_instance_variant(
    context: &CookingBuildContext<'_>,
    release_agent_id: Uuid,
    blog_repository_id: RepositoryId,
    parameters: Vec<ParameterValue>,
    name: &str,
    operation_suffix: &str,
) -> Result<PreparedCookingInstance, BuildError> {
    if name.is_empty() || operation_suffix.is_empty() {
        return Err(invalid_state("cooking instance variant identity is empty"));
    }
    let instance_client = rpc_instance_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/ImportAgent",
    )?;
    let imported = instance_client
        .import_agent(ImportAgentRequest {
            context: mutation_context(&format!("import-{operation_suffix}")).into(),
            project_id: opaque(context.project_id.as_uuid()).into(),
            release_agent_id: opaque(release_agent_id).into(),
            // InstanceName is a bounded lowercase key, so keep the display
            // name in the same canonical form used by the release manifest.
            name: name.to_owned(),
            parameters,
            selected_policy: RuntimePolicy {
                vcpus: 1,
                memory_mib: 256,
                network: NetworkPolicy::BrokerOnly.into(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await
        .map_err(|error| format!("ImportAgent RPC failed: {error}"))?
        .into_owned();
    let instance_id = response_id(imported.instance_id.into_option(), "ImportAgent instance")?;
    let revision_id = response_id(imported.revision_id.into_option(), "ImportAgent revision")?;

    let instance_client = rpc_instance_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/CreateAttachment",
    )?;
    let attachment = instance_client
        .create_attachment(CreateAttachmentRequest {
            context: mutation_context(&format!("attach-{operation_suffix}-blog")).into(),
            instance_id: opaque(instance_id).into(),
            repository_id: opaque(blog_repository_id.as_uuid()).into(),
            ref_selector: RefSelector {
                selector: Some(ref_selector::Selector::Exact(String::from(
                    "refs/heads/main",
                ))),
                ..Default::default()
            }
            .into(),
            // The cooking mailbox drives execution; a blog source push must
            // not create an unrelated agent run before ingress is tested.
            trigger_policy: TriggerPolicy::Manual.into(),
            ..Default::default()
        })
        .await
        .map_err(|error| format!("CreateAttachment RPC failed: {error}"))?
        .into_owned();
    let attachment_id = response_id(attachment.attachment_id.into_option(), "CreateAttachment")?;

    let instance_client = rpc_instance_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/CreateMailbox",
    )?;
    let mailbox = instance_client
        .create_mailbox(CreateMailboxRequest {
            context: mutation_context(&format!("create-{operation_suffix}-mailbox")).into(),
            instance_id: opaque(instance_id).into(),
            ..Default::default()
        })
        .await
        .map_err(|error| format!("CreateMailbox RPC failed: {error}"))?
        .into_owned();
    let mailbox_id = response_id(mailbox.mailbox_id.into_option(), "CreateMailbox")?;
    Ok(PreparedCookingInstance {
        instance_id,
        revision_id,
        attachment_id,
        mailbox_id,
    })
}

/// Installs a published gateway declaration through the production gateway
/// command. A later binding command supplies its runtime mailbox authority.
pub async fn install_cooking_gateway(
    context: &CookingBuildContext<'_>,
    release_id: Uuid,
    repository_id: RepositoryId,
) -> Result<InstalledCookingGateway, BuildError> {
    let client = rpc_gateway_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.gateway.v1.GatewayService/InstallReleaseGateways",
    )?;
    client
        .install_release_gateways(InstallReleaseGatewaysRequest {
            context: mutation_context("install-cooking-gateway").into(),
            release_id: opaque(release_id).into(),
            ..Default::default()
        })
        .await?;
    let client = rpc_gateway_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.gateway.v1.GatewayService/ListProjectGateways",
    )?;
    let listed = client
        .list_project_gateways(ListProjectGatewaysRequest {
            project_id: opaque(context.project_id.as_uuid()).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let gateway = listed
        .gateways
        .into_iter()
        .find(|gateway| {
            gateway
                .repository_id
                .as_option()
                .is_some_and(|id| id.value == repository_id.to_string())
        })
        .ok_or_else(|| invalid_state("installed cooking gateway was not listed"))?;
    let gateway_id = response_id(gateway.id.into_option(), "ListProjectGateways gateway")?;
    let client = rpc_gateway_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.gateway.v1.GatewayService/GetGateway",
    )?;
    let current = client
        .get_gateway(GetGatewayRequest {
            gateway_id: opaque(gateway_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let revision_id = response_id(
        current
            .gateway
            .into_option()
            .and_then(|summary| summary.active_revision_id.into_option()),
        "GetGateway active revision",
    )?;
    Ok(InstalledCookingGateway {
        gateway_id,
        revision_id,
    })
}

/// Configures the installed gateway and binds its declared publication slot
/// to the mailbox created by [`prepare_cooking_instance`].
#[allow(clippy::too_many_lines)] // Keep the RPC receipt/replay proof in order.
pub async fn configure_cooking_gateway(
    context: &CookingBuildContext<'_>,
    gateway: InstalledCookingGateway,
    parameters: Vec<ParameterValue>,
    inbound_import_id: Uuid,
    inbound_secret_version_id: Uuid,
    mailbox_id: Uuid,
) -> Result<ConfiguredCookingGateway, BuildError> {
    let configure_key = format!("cooking-build-configure-{}", Uuid::new_v4());
    let configure_parameters = parameters;
    let configure_secret_selections = vec![GatewaySecretSelection {
        slot_key: String::from("webhook"),
        import_id: opaque(inbound_import_id).into(),
        secret_version_id: opaque(inbound_secret_version_id).into(),
        route_path: String::from("/cooking/telegram"),
        header_name: String::from("x-telegram-bot-api-secret-token"),
        ..Default::default()
    }];
    let client = rpc_gateway_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.gateway.v1.GatewayService/ConfigureGateway",
    )?;
    let configured = client
        .configure_gateway(ConfigureGatewayRequest {
            context: mutation_context_with_key(&configure_key).into(),
            gateway_id: opaque(gateway.gateway_id).into(),
            expected_revision_id: opaque(gateway.revision_id).into(),
            parameters: configure_parameters.clone(),
            secret_selections: configure_secret_selections.clone(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let revision_id = response_id(configured.revision_id.into_option(), "ConfigureGateway")?;
    // A mailbox may bind a producer only once across immutable gateway
    // revisions. Tie each fresh producer identity to its exact revision; the
    // same value is reused for the binding replay below.
    let producer_id = format!("cooking-gateway-{revision_id}");
    let configure_receipt = configured
        .receipt
        .as_option()
        .ok_or_else(|| invalid_state("ConfigureGateway returned no receipt"))?
        .clone();
    let mut replay_context = mutation_context_with_key(&configure_key);
    replay_context.request_id = opaque(Uuid::new_v4()).into();
    let configure_replay = client
        .configure_gateway(ConfigureGatewayRequest {
            context: replay_context.into(),
            gateway_id: opaque(gateway.gateway_id).into(),
            expected_revision_id: opaque(gateway.revision_id).into(),
            parameters: configure_parameters,
            secret_selections: configure_secret_selections,
            ..Default::default()
        })
        .await?
        .into_owned();
    assert_eq!(
        response_id(
            configure_replay.revision_id.into_option(),
            "ConfigureGateway replay",
        )?,
        revision_id,
        "ConfigureGateway retry must replay the original immutable revision"
    );
    assert_eq!(
        configure_replay.receipt.as_option(),
        Some(&configure_receipt),
        "ConfigureGateway retry must return the original receipt"
    );
    let client = rpc_gateway_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.gateway.v1.GatewayService/CreateMailboxBinding",
    )?;
    // A configured revision is immutable, so every invocation needs its own
    // command identity.  Reusing a process-wide key would make the restore
    // phase look like a conflicting retry of the adversarial binding.
    let binding_key = format!("cooking-build-bind-gateway-mailbox-{}", Uuid::new_v4());
    let binding = client
        .create_mailbox_binding(CreateMailboxBindingRequest {
            context: mutation_context_with_key(&binding_key).into(),
            gateway_revision_id: opaque(revision_id).into(),
            slot_key: String::from("cooking_requests"),
            mailbox_id: opaque(mailbox_id).into(),
            producer_id: producer_id.clone(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let binding_value = binding
        .binding
        .as_option()
        .ok_or_else(|| invalid_state("CreateMailboxBinding returned no binding"))?;
    let binding_receipt = binding
        .receipt
        .as_option()
        .ok_or_else(|| invalid_state("CreateMailboxBinding returned no receipt"))?
        .clone();
    let binding_replay = client
        .create_mailbox_binding(CreateMailboxBindingRequest {
            context: mutation_context_with_key(&binding_key).into(),
            gateway_revision_id: opaque(revision_id).into(),
            slot_key: String::from("cooking_requests"),
            mailbox_id: opaque(mailbox_id).into(),
            producer_id,
            ..Default::default()
        })
        .await?
        .into_owned();
    let binding_replay_value = binding_replay
        .binding
        .as_option()
        .ok_or_else(|| invalid_state("CreateMailboxBinding replay returned no binding"))?;
    assert_eq!(
        binding_replay_value.id, binding_value.id,
        "CreateMailboxBinding retry must replay the original binding"
    );
    assert_eq!(
        binding_replay.receipt.as_option(),
        Some(&binding_receipt),
        "CreateMailboxBinding retry must return the original receipt"
    );
    let grant_id = response_id(
        binding_value.clone().grant_id.into_option(),
        "CreateMailboxBinding grant",
    )?;
    Ok(ConfiguredCookingGateway {
        revision_id,
        grant_id,
    })
}

/// Ordinary published agent releases used by the update lifecycle scenario.
#[derive(Debug, Clone)]
pub struct PublishedCookingUpdateBuilds {
    /// Candidate whose hook runs the v1-to-v2 `SQLite` migration.
    pub migrate: PublishedCookingRepository,
    /// Candidate whose hook deliberately returns a nonzero exit after its
    /// transactional migration rollback.
    pub rollback: PublishedCookingRepository,
    /// Candidate whose hook terminates by signal, requiring operator recovery.
    pub abnormal: PublishedCookingRepository,
}

/// Copies, pushes, builds, versions, and publishes the canonical cooking
/// gateway and agent sources.
///
/// Repository metadata is created through the trusted fixture factory.  Build
/// and release rows are created by the production Git receive/build workers;
/// this helper only observes them and invokes the existing release RPCs.
pub async fn build_and_publish(
    context: CookingBuildContext<'_>,
) -> Result<PublishedCookingBuilds, BuildError> {
    let gateway = build_one(
        &context,
        "cooking-gateway",
        canonical_source(context.source_root, "cooking-gateway"),
        "Cooking gateway",
        false,
    )
    .await?;
    let agent = build_one(
        &context,
        "cooking-agent",
        canonical_source(context.source_root, "cooking-agent"),
        "Cooking agent",
        true,
    )
    .await?;
    Ok(PublishedCookingBuilds { gateway, agent })
}

/// Builds a distinct cooking-agent release through the ordinary
/// Git/build/publish path. The guest keeps its declared model rule but sends
/// that request to the relay origin, so the host must deny the mismatch.
pub async fn build_and_publish_adversarial_agent(
    context: &CookingBuildContext<'_>,
    canonical: &PublishedCookingRepository,
) -> Result<PublishedCookingRepository, BuildError> {
    let source = context
        .root
        .join(format!("cooking-adversarial-agent-{}", Uuid::new_v4()));
    copy_source_tree(&canonical.source_path, &source)?;
    let source_file = source.join("cooking_agent.py");
    let source_code = fs::read_to_string(&source_file)?;
    let canonical_call = "destination = 'api.model.example' if model else 'relay.cooking.example'";
    let adversarial_call = "destination = 'relay.cooking.example'";
    let mutated = source_code.replacen(canonical_call, adversarial_call, 1);
    if mutated == source_code {
        return Err(invalid_state(
            "canonical cooking agent broker call changed unexpectedly",
        ));
    }
    fs::write(source_file, mutated)?;
    build_one(
        context,
        "cooking-agent-adversarial",
        source,
        "Cooking agent adversarial destination probe",
        true,
    )
    .await
}

/// Builds the canonical cooking agent after applying the checked-in guest
/// crash transformer. The transformed source is pushed and observed through
/// the same Git, build, and release workers as every ordinary fixture build.
pub async fn build_and_publish_guest_crash_agent(
    context: &CookingBuildContext<'_>,
    canonical: &PublishedCookingRepository,
) -> Result<PublishedCookingRepository, BuildError> {
    let source = context
        .root
        .join(format!("cooking-agent-guest-crash-{}", Uuid::new_v4()));
    copy_source_tree(&canonical.source_path, &source)?;
    let transformer = context.source_root.join("tests/guest_crash.py");
    let runtime_probe = context.source_root.join("tests/guest_confinement.py");
    let descriptor_staging = tempfile::Builder::new()
        .prefix(".guest-crash-probe-")
        .tempdir_in(context.root)?;
    let descriptor_root = descriptor_staging.path();
    let mut candidate_arguments = Vec::new();
    for (label, value) in [
        ("inbound", super::cooking::INBOUND_SENTINEL),
        ("model", super::cooking::MODEL_SENTINEL),
        ("relay", super::cooking::RELAY_SENTINEL),
        ("inbound_rotated", super::cooking::INBOUND_ROTATED_SENTINEL),
        ("model_rotated", super::cooking::MODEL_ROTATED_SENTINEL),
        ("relay_rotated", super::cooking::RELAY_ROTATED_SENTINEL),
    ] {
        let candidate = descriptor_root.join(label);
        fs::write(&candidate, value.as_bytes())?;
        candidate_arguments.push(format!("{label}={}", candidate.display()));
    }
    let descriptor = descriptor_root.join("descriptors.json");
    let mut generator = Command::new("python3");
    generator
        .arg(context.source_root.join("tests/guest_confinement_probe.py"))
        .arg("--output")
        .arg(&descriptor);
    for candidate in &candidate_arguments {
        generator.arg("--candidate").arg(candidate);
    }
    let generated = generator.status().await?;
    if !generated.success() {
        return Err(invalid_state(
            "guest confinement descriptor generation failed",
        ));
    }
    let status = Command::new("python3")
        .arg(&transformer)
        .arg(source.join("cooking_agent.py"))
        .arg(source.join("cooking_agent.py"))
        .arg("--runtime-source")
        .arg(&runtime_probe)
        .arg("--descriptor-source")
        .arg(&descriptor)
        .status()
        .await?;
    if !status.success() {
        return Err(invalid_state("guest crash source transformation failed"));
    }
    drop(descriptor_staging);
    build_one(
        context,
        "cooking-agent-guest-crash",
        source,
        "Cooking agent deterministic guest crash probe",
        true,
    )
    .await
}

/// Builds a release whose handler targets an undeclared mailbox slot.
///
/// It reuses the canonical gateway repository so release-family and route
/// identity remain the same while installation exercises a real new release.
pub async fn build_and_publish_adversarial_gateway(
    context: &CookingBuildContext<'_>,
    canonical: &PublishedCookingRepository,
) -> Result<PublishedCookingRepository, BuildError> {
    let source = context
        .root
        .join(format!("cooking-adversarial-gateway-{}", Uuid::new_v4()));
    copy_source_tree(&canonical.source_path, &source)?;
    initialize_git(&source, "Cooking gateway adversarial foreign-slot probe").await?;
    let remote = format!(
        "http://{}/{}",
        context.running.http_addr(),
        canonical.repository_id
    );
    git(&source, &["remote", "add", "origin", &remote]).await?;
    // This repository already contains the canonical release source.  Base
    // the adversarial commit on that remote head so the ordinary Git receive
    // fast-forward check remains enabled.
    authenticated_git(
        &source,
        context.identity.git_token,
        &["fetch", "origin", "refs/heads/main"],
    )
    .await?;
    git(&source, &["reset", "--hard", "FETCH_HEAD"]).await?;
    let source_path = source.join("src/main.rs");
    let source_code = fs::read_to_string(&source_path)?;
    let canonical_slot = "const COOKING_REQUESTS_SLOT: &str = \"cooking_requests\";";
    let foreign_slot =
        format!("const COOKING_REQUESTS_SLOT: &str = \"{FOREIGN_PUBLICATION_SLOT}\";");
    let adversarial_code = source_code.replace(canonical_slot, &foreign_slot);
    if adversarial_code == source_code {
        return Err(invalid_state(
            "canonical cooking gateway slot declaration changed unexpectedly",
        ));
    }
    fs::write(&source_path, adversarial_code)?;
    build_one_in_repository(
        context,
        "cooking-gateway-adversarial",
        source,
        "Cooking gateway adversarial foreign-slot probe",
        canonical.repository_id,
        true,
        false,
    )
    .await
}

/// Creates and pushes the separate blog repository used by the cooking
/// instance attachment.
pub async fn create_cooking_blog_repository(
    context: &CookingBuildContext<'_>,
) -> Result<PreparedCookingBlog, BuildError> {
    let source = canonical_source(context.source_root, "cooking-blog");
    create_cooking_blog_toolchain_repository(context, &source).await?;
    let repository = context
        .repositories
        .create_repository_trusted(&CreateRepository {
            project_id: context.project_id,
            name: format!("cooking-blog-{}", Uuid::new_v4()),
            default_branch: GitRef::parse("refs/heads/main")?,
            is_public: false,
            agent_runs_enabled: false,
        })
        .await?;
    let destination = context
        .root
        .join(format!("cooking-blog-{}", Uuid::new_v4()));
    copy_source_tree(&source, &destination)?;
    // The Hugo toolchain is a separate same-project image repository. Keep
    // Dockerfile, vendor inputs, and heph.images out of the site repository so
    // ordinary content commits do not enqueue a new OCI production job.
    fs::remove_file(destination.join("Dockerfile"))?;
    fs::remove_file(destination.join("heph.images.toml"))?;
    fs::remove_file(destination.join("verify-hugo.sh"))?;
    fs::remove_dir_all(destination.join("vendor"))?;
    initialize_git(&destination, "Cooking blog").await?;
    let remote = format!("http://{}/{}", context.running.http_addr(), repository.id);
    git(&destination, &["remote", "add", "origin", &remote]).await?;
    authenticated_git(
        &destination,
        context.identity.git_token,
        &["push", "origin", "HEAD:refs/heads/main"],
    )
    .await?;
    let source_commit = git_output(&destination, &["rev-parse", "HEAD"]).await;
    Ok(PreparedCookingBlog {
        repository_id: repository.id,
        source_commit: source_commit?,
    })
}

async fn create_cooking_blog_toolchain_repository(
    context: &CookingBuildContext<'_>,
    source: &Path,
) -> Result<(), BuildError> {
    let repository = context
        .repositories
        .create_repository_trusted(&CreateRepository {
            project_id: context.project_id,
            name: format!("cooking-blog-toolchain-{}", Uuid::new_v4()),
            default_branch: GitRef::parse("refs/heads/main")?,
            is_public: false,
            agent_runs_enabled: false,
        })
        .await?;
    let destination = context
        .root
        .join(format!("cooking-blog-toolchain-{}", Uuid::new_v4()));
    fs::create_dir_all(destination.join("vendor"))?;
    for name in ["Dockerfile", "heph.images.toml", "verify-hugo.sh"] {
        fs::copy(source.join(name), destination.join(name))?;
    }
    copy_source_tree(&source.join("vendor"), &destination.join("vendor"))?;
    initialize_git(&destination, "Cooking blog Hugo toolchain").await?;
    let remote = format!("http://{}/{}", context.running.http_addr(), repository.id);
    git(&destination, &["remote", "add", "origin", &remote]).await?;
    authenticated_git(
        &destination,
        context.identity.git_token,
        &["push", "origin", "HEAD:refs/heads/main"],
    )
    .await?;
    wait_for_project_image(
        context.pool,
        repository.id,
        "cooking-blog-hugo",
        context.timeout,
    )
    .await
}

async fn wait_for_project_image(
    pool: &PgPool,
    repository_id: RepositoryId,
    key: &str,
    timeout: Duration,
) -> Result<(), BuildError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT status, failure_reason, image_reference
               FROM repository_oci_image_definitions
              WHERE source_repository_id = $1 AND key = $2
              ORDER BY updated_at DESC, id DESC
              LIMIT 1",
        )
        .bind(repository_id.as_uuid())
        .bind(key)
        .fetch_optional(pool)
        .await?;
        match row {
            Some((status, _failure_reason, Some(reference))) if status == "ready" => {
                let digest_pinned = reference
                    .rsplit_once("@sha256:")
                    .is_some_and(|(_, digest)| {
                        digest.len() == 64
                            && digest
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                    });
                if !digest_pinned {
                    return Err(invalid_state(
                        "project image became ready without a digest-pinned reference",
                    ));
                }
                return Ok(());
            }
            Some((status, failure_reason, _)) if status == "failed" => {
                return Err(invalid_state(&format!(
                    "cooking blog project image failed: {}",
                    failure_reason.unwrap_or_else(|| String::from("unspecified failure")),
                )));
            }
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid_state(
                "timed out waiting for cooking blog project image readiness",
            ));
        }
        sleep(Duration::from_millis(500)).await;
    }
}

/// Builds and publishes the three explicit update-hook variants through the
/// production source/build/release path.  The source copies are test-owned
/// variants under the temporary fixture root; the canonical checkout is never
/// modified.
pub async fn build_and_publish_update_variants(
    context: CookingBuildContext<'_>,
    base: &PublishedCookingRepository,
) -> Result<PublishedCookingUpdateBuilds, BuildError> {
    // Reuse the canonical agent repository so all candidates remain in its
    // production-created family and satisfy CreateUpdate's family check.
    let migrate_source = base.working_path.clone();
    fs::write(
        migrate_source.join(".cooking-update-migrate"),
        b"canonical v2 migration candidate\n",
    )?;
    let migrate = build_one_in_repository(
        &context,
        "cooking-agent-update-migrate",
        migrate_source,
        "Cooking agent update migration",
        base.repository_id,
        true,
        true,
    )
    .await?;
    let rollback_source = migrate.working_path.clone();
    apply_update_variant(&rollback_source, UpdateVariant::Rollback)?;
    let rollback = build_one_in_repository(
        &context,
        "cooking-agent-update-rollback",
        rollback_source,
        "Cooking agent update rollback",
        base.repository_id,
        true,
        true,
    )
    .await?;
    let abnormal_source = migrate.working_path.clone();
    apply_update_variant(&abnormal_source, UpdateVariant::Abnormal)?;
    let abnormal = build_one_in_repository(
        &context,
        "cooking-agent-update-abnormal",
        abnormal_source,
        "Cooking agent update abnormal",
        base.repository_id,
        true,
        true,
    )
    .await?;
    Ok(PublishedCookingUpdateBuilds {
        migrate,
        rollback,
        abnormal,
    })
}

// Keep source publication and the resulting RPC mutations in one ordered
// sequence so returned provenance cannot describe a partially published pair.
#[allow(clippy::too_many_lines)]
async fn build_one(
    context: &CookingBuildContext<'_>,
    key: &str,
    source: PathBuf,
    display_name: &str,
    expect_update_hook: bool,
) -> Result<PublishedCookingRepository, BuildError> {
    let repository = context
        .repositories
        .create_repository_trusted(&CreateRepository {
            project_id: context.project_id,
            name: format!("{key}-{}", Uuid::new_v4()),
            default_branch: GitRef::parse("refs/heads/main")?,
            is_public: false,
            // This helper creates build-only repositories.  The scenario
            // attaches the resulting release explicitly before running it.
            agent_runs_enabled: false,
        })
        .await?;
    build_one_in_repository(
        context,
        key,
        source,
        display_name,
        repository.id,
        false,
        expect_update_hook,
    )
    .await
}

// Every release in one update sequence must retain the production-created
// family identity.  Building successive commits in one repository gives the
// release worker that family continuity without mutating release metadata.
#[allow(clippy::too_many_lines)]
async fn build_one_in_repository(
    context: &CookingBuildContext<'_>,
    key: &str,
    source: PathBuf,
    display_name: &str,
    repository_id: RepositoryId,
    reuse_checkout: bool,
    expect_update_hook: bool,
) -> Result<PublishedCookingRepository, BuildError> {
    let source_path = if reuse_checkout {
        source
    } else {
        let destination = context
            .root
            .join(format!("cooking-build-{key}-{}", Uuid::new_v4()));
        copy_source_tree(&source, &destination)?;
        initialize_git(&destination, display_name).await?;
        destination
    };
    if reuse_checkout {
        git(&source_path, &["add", "--all"]).await?;
        git(&source_path, &["commit", "--message", display_name]).await?;
    }
    let source_commit = git_output(&source_path, &["rev-parse", "HEAD"]).await?;
    let remote = format!("http://{}/{}", context.running.http_addr(), repository_id);
    if !reuse_checkout {
        git(&source_path, &["remote", "add", "origin", &remote]).await?;
    }
    authenticated_git(
        &source_path,
        context.identity.git_token,
        &["push", "origin", "HEAD:refs/heads/main"],
    )
    .await?;

    let build_request_id =
        wait_for_build_row(context.pool, repository_id, &source_commit, context.timeout).await?;
    let build = wait_for_successful_build(
        context.running,
        context.identity.rpc_token,
        build_request_id,
        context.timeout,
    )
    .await?;
    let release_id = build
        .release_id
        .into_option()
        .ok_or_else(|| invalid_state("successful build has no release"))?
        .value
        .parse::<Uuid>()?;
    let draft_release = wait_for_draft_release(
        context.running,
        context.identity.rpc_token,
        release_id,
        context.timeout,
    )
    .await?;
    let draft_agent = draft_release
        .agents
        .first()
        .ok_or_else(|| invalid_state("GetRelease returned no cooking agent"))?;
    if expect_update_hook && draft_agent.update_hook.as_option().is_none() {
        return Err(invalid_state(
            "GetRelease omitted the canonical cooking update hook",
        ));
    }

    let version = format!("v1.0.0-{}", &source_commit[..12]);
    let release_client = rpc_release_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.release.v1.ReleaseService/SetDraftVersion",
    )?;
    let draft = release_client
        .set_draft_version(SetDraftVersionRequest {
            context: mutation_context("set-draft-version").into(),
            release_id: opaque(release_id).into(),
            version: version.clone(),
            ..Default::default()
        })
        .await?
        .into_owned()
        .release
        .into_option()
        .ok_or_else(|| invalid_state("SetDraftVersion returned no release"))?;
    if draft.state.to_i32() != 1 {
        return Err(invalid_state(
            "SetDraftVersion did not leave a draft release",
        ));
    }
    let release_client = rpc_release_client(
        context.running,
        context.identity.rpc_token,
        "/hephaestus.release.v1.ReleaseService/PublishRelease",
    )?;
    let published = release_client
        .publish_release(PublishReleaseRequest {
            context: mutation_context("publish-release").into(),
            release_id: opaque(release_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned()
        .release
        .into_option()
        .ok_or_else(|| invalid_state("PublishRelease returned no release"))?;
    if published.state.to_i32() != 2 {
        return Err(invalid_state("PublishRelease did not publish the release"));
    }

    let release_agent_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM release_agents
          WHERE release_id = $1 ORDER BY id LIMIT 1",
    )
    .bind(release_id)
    .fetch_one(context.pool)
    .await?;

    let provenance_path = context
        .root
        .join(format!("cooking-build-provenance-{key}-{}", Uuid::new_v4()));
    copy_source_tree(&source_path, &provenance_path)?;
    Ok(PublishedCookingRepository {
        repository_id,
        actor_id: context.identity.actor.user_id,
        source_commit,
        build_request_id,
        release_id,
        release_agent_id,
        version,
        build_definition_hash: build.build_definition_hash,
        configuration_hash: build.configuration_hash,
        manifest_hash: published.manifest_hash,
        source_path: provenance_path,
        working_path: source_path,
    })
}

#[derive(Clone, Copy)]
enum UpdateVariant {
    Rollback,
    Abnormal,
}

fn apply_update_variant(source_path: &Path, variant: UpdateVariant) -> Result<(), BuildError> {
    let config_path = source_path.join("agent.toml");
    let config = fs::read_to_string(&config_path)?;
    let arguments = match variant {
        UpdateVariant::Rollback => "arguments = [\"--rollback-fixture\"]",
        UpdateVariant::Abnormal => "arguments = [\"--abnormal-fixture\"]",
    };
    // Variants are applied sequentially to one production family checkout.
    // Replace whichever prior hook marker is present so the abnormal release
    // cannot accidentally retain the rollback hook's nonzero exit behavior.
    let config = config
        .replace("arguments = [\"--migrate\"]", arguments)
        .replace("arguments = [\"--rollback-fixture\"]", arguments);
    fs::write(config_path, config)?;
    if matches!(variant, UpdateVariant::Rollback) {
        let python_path = source_path.join("cooking_agent.py");
        let source = fs::read_to_string(&python_path)?;
        let source = source.replace(
            "if version == 2:\n            return\n",
            "if version == 2 and fail:\n            raise ValueError('deliberate migration rollback from v2')\n        if version == 2:\n            return\n",
        );
        fs::write(python_path, source)?;
    }
    if matches!(variant, UpdateVariant::Abnormal) {
        let python_path = source_path.join("cooking_agent.py");
        let source = fs::read_to_string(&python_path)?;
        let source = source
            .replace("import argparse\n", "import argparse\nimport os\nimport signal\n")
            .replace(
                "parser.add_argument('--rollback-fixture', action='store_true')",
                "parser.add_argument('--rollback-fixture', action='store_true')\n    parser.add_argument('--abnormal-fixture', action='store_true')",
            )
            .replace(
                "if args.migrate or args.rollback_fixture:\n",
                "if args.abnormal_fixture:\n        os.kill(os.getpid(), signal.SIGKILL)\n    if args.migrate or args.rollback_fixture:\n",
            );
        fs::write(python_path, source)?;
    }
    Ok(())
}

fn rpc_build_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
) -> Result<BuildServiceClient<connectrpc::client::HttpClient>, BuildError> {
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!(
                "Bearer {}",
                token_factory("/hephaestus.build.v1.BuildService/GetBuild")
            ))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    let transport = connectrpc::client::HttpClient::plaintext();
    Ok(BuildServiceClient::new(transport, config))
}

fn rpc_gateway_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<GatewayServiceClient<connectrpc::client::HttpClient>, BuildError> {
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", token_factory(audience)))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok(GatewayServiceClient::new(
        connectrpc::client::HttpClient::plaintext(),
        config,
    ))
}

fn rpc_instance_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<AgentInstanceServiceClient<connectrpc::client::HttpClient>, BuildError> {
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", token_factory(audience)))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok(AgentInstanceServiceClient::new(
        connectrpc::client::HttpClient::plaintext(),
        config,
    ))
}

fn rpc_release_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<ReleaseServiceClient<connectrpc::client::HttpClient>, BuildError> {
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", token_factory(audience)))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok(ReleaseServiceClient::new(
        connectrpc::client::HttpClient::plaintext(),
        config,
    ))
}

async fn wait_for_build_row(
    pool: &PgPool,
    repository_id: RepositoryId,
    source_commit: &str,
    timeout: Duration,
) -> Result<Uuid, BuildError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(id) = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM build_requests
             WHERE repository_id = $1 AND source_commit = $2
               AND source_ref = 'refs/heads/main'
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(repository_id.as_uuid())
        .bind(source_commit)
        .fetch_optional(pool)
        .await?
        {
            return Ok(id);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid_state("Git push did not create a build request"));
        }
        sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_for_successful_build(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    build_request_id: Uuid,
    timeout: Duration,
) -> Result<rpc_proto::messages::hephaestus::build::v1::Build, BuildError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let client = rpc_build_client(running, token_factory)?;
        let build = client
            .get_build(GetBuildRequest {
                build_id: opaque(build_request_id).into(),
                ..Default::default()
            })
            .await?
            .into_owned()
            .build
            .into_option()
            .ok_or_else(|| invalid_state("GetBuild returned no build"))?;
        match build.state.to_i32() {
            value if value == BuildState::BUILD_STATE_SUCCEEDED as i32 => return Ok(build),
            value
                if value == BuildState::BUILD_STATE_FAILED as i32
                    || value == BuildState::BUILD_STATE_CANCELLED as i32 =>
            {
                let logs = build
                    .logs
                    .iter()
                    .map(|line| redact_build_log(line))
                    .collect::<Vec<_>>()
                    .join("\\n");
                return Err(invalid_state(&format!(
                    "cooking isolated build failed: code={} exit={:?} logs={logs}",
                    build.failure_code, build.exit_code
                )));
            }
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid_state(
                "timed out waiting for cooking isolated build",
            ));
        }
        sleep(Duration::from_millis(500)).await;
    }
}

async fn wait_for_draft_release(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    release_id: Uuid,
    timeout: Duration,
) -> Result<rpc_proto::messages::hephaestus::release::v1::Release, BuildError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let client = rpc_release_client(
            running,
            token_factory,
            "/hephaestus.release.v1.ReleaseService/GetRelease",
        )?;
        let release = client
            .get_release(GetReleaseRequest {
                release_id: opaque(release_id).into(),
                ..Default::default()
            })
            .await?
            .into_owned()
            .release
            .into_option()
            .ok_or_else(|| invalid_state("GetRelease returned no release"))?;
        if release.state.to_i32() == 1 {
            return Ok(release);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(invalid_state("timed out waiting for draft cooking release"));
        }
        sleep(Duration::from_millis(500)).await;
    }
}

fn mutation_context(operation: &str) -> RequestContext {
    mutation_context_with_key(&format!("cooking-build-{operation}-{}", Uuid::new_v4()))
}

fn mutation_context_with_key(key: &str) -> RequestContext {
    RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: key.to_owned(),
        ..Default::default()
    }
}

fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

fn response_id(value: Option<OpaqueId>, operation: &str) -> Result<Uuid, BuildError> {
    value
        .ok_or_else(|| invalid_state(&format!("{operation} returned no ID")))?
        .value
        .parse::<Uuid>()
        .map_err(Into::into)
}

fn canonical_source(source_root: &Path, name: &str) -> PathBuf {
    source_root.join(name)
}

fn copy_source_tree(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "canonical cooking source is not a directory: {}",
                source.display()
            ),
        ));
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" || name == "target" || name == "__pycache__" {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(&name);
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.is_file()
            && matches!(
                source_path.extension().and_then(std::ffi::OsStr::to_str),
                Some("pyc" | "pyo")
            )
        {
            continue;
        }
        if metadata.is_dir() {
            copy_source_tree(&source_path, &destination_path)?;
        } else if metadata.file_type().is_symlink() {
            unix_fs::symlink(fs::read_link(source_path)?, destination_path)?;
        } else if metadata.is_file() {
            fs::copy(source_path, destination_path)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "canonical cooking source contains unsupported file type",
            ));
        }
    }
    Ok(())
}

async fn initialize_git(source: &Path, display_name: &str) -> Result<(), BuildError> {
    git(source, &["init", "--initial-branch=main"]).await?;
    git(
        source,
        &["config", "user.email", "cooking-fixture@example.invalid"],
    )
    .await?;
    git(source, &["config", "user.name", display_name]).await?;
    git(source, &["add", "--all"]).await?;
    git(source, &["commit", "--message", "canonical cooking source"]).await?;
    Ok(())
}

async fn git(directory: &Path, arguments: &[&str]) -> Result<(), BuildError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .output()
        .await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(invalid_state(&format!(
            "Git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

async fn git_output(directory: &Path, arguments: &[&str]) -> Result<String, BuildError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .output()
        .await?;
    if !output.status.success() {
        return Err(invalid_state(
            "Git did not produce the cooking source commit",
        ));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

async fn authenticated_git(
    directory: &Path,
    token: &str,
    arguments: &[&str],
) -> Result<(), BuildError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Bearer {token}"),
        )
        .stdin(Stdio::null())
        .output()
        .await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(invalid_state(&format!(
            "authenticated Git push failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn invalid_state(message: &str) -> BuildError {
    io::Error::other(message.to_owned()).into()
}

fn redact_build_log(line: &str) -> String {
    line.replace("golden-brokered-provider-sentinel-5d1a", "[REDACTED]")
        .replace("cooking-inbound-only-fixture-sentinel", "[REDACTED]")
        .replace("cooking-model-only-fixture-sentinel-724c", "[REDACTED]")
        .replace("cooking-relay-only-fixture-sentinel-819e", "[REDACTED]")
        .replace("cooking-model-rotated-fixture-sentinel-936f", "[REDACTED]")
        .replace(
            "cooking-inbound-rotated-fixture-sentinel-157a",
            "[REDACTED]",
        )
        .replace("cooking-relay-rotated-fixture-sentinel-482b", "[REDACTED]")
        .replace("telegram-bot-api-secret-token", "[REDACTED]")
}

#[cfg(test)]
mod tests {
    use super::{UpdateVariant, apply_update_variant, copy_source_tree, redact_build_log};
    use std::{fs, os::unix::fs::PermissionsExt};
    use uuid::Uuid;

    #[test]
    fn source_copy_excludes_generated_and_git_directories() {
        let root = std::env::temp_dir().join(format!("cooking-build-helper-{}", Uuid::new_v4()));
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir_all(source.join("vendor")).expect("vendor directory");
        fs::create_dir_all(source.join(".cargo")).expect("cargo directory");
        fs::create_dir_all(source.join("target")).expect("target directory");
        fs::create_dir_all(source.join("__pycache__")).expect("Python cache directory");
        fs::create_dir_all(source.join("nested/.git")).expect("nested git directory");
        fs::write(source.join("agent.toml"), b"agent").expect("agent config");
        fs::write(source.join("vendor/config"), b"vendored").expect("vendor file");
        fs::write(source.join(".cargo/config.toml"), b"cargo").expect("cargo file");
        fs::write(source.join("target/stale"), b"stale").expect("target file");
        fs::write(source.join("__pycache__/cooking.cpython-314.pyc"), b"stale")
            .expect("Python cache file");
        fs::write(source.join("generated.pyc"), b"stale").expect("Python bytecode file");
        fs::write(source.join("nested/.git/stale"), b"stale").expect("git file");
        fs::set_permissions(source.join("agent.toml"), fs::Permissions::from_mode(0o640))
            .expect("source mode");

        copy_source_tree(&source, &destination).expect("copy source");
        assert_eq!(
            fs::read(destination.join("agent.toml")).expect("agent"),
            b"agent"
        );
        assert_eq!(
            fs::read(destination.join("vendor/config")).expect("vendor"),
            b"vendored"
        );
        assert_eq!(
            fs::read(destination.join(".cargo/config.toml")).expect("cargo"),
            b"cargo"
        );
        assert!(!destination.join("target").exists());
        assert!(!destination.join("__pycache__").exists());
        assert!(!destination.join("generated.pyc").exists());
        assert!(!destination.join("nested/.git").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn canonical_cooking_blog_dockerfile_satisfies_production_policy() {
        let dockerfile = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/cooking/cooking-blog/Dockerfile");
        let source = fs::read_to_string(dockerfile).expect("canonical cooking blog Dockerfile");
        oci_builder_worker::DockerfilePolicy::validate(&source)
            .expect("canonical cooking blog Dockerfile policy");
    }

    #[test]
    fn build_log_redaction_covers_all_fixture_credentials() {
        let raw = concat!(
            "golden-brokered-provider-sentinel-5d1a ",
            "cooking-inbound-only-fixture-sentinel ",
            "cooking-model-only-fixture-sentinel-724c ",
            "cooking-relay-only-fixture-sentinel-819e ",
            "cooking-model-rotated-fixture-sentinel-936f ",
            "cooking-inbound-rotated-fixture-sentinel-157a ",
            "cooking-relay-rotated-fixture-sentinel-482b ",
            "telegram-bot-api-secret-token"
        );
        let redacted = redact_build_log(raw);
        assert!(!redacted.contains("sentinel"));
        assert!(!redacted.contains("telegram-bot-api-secret-token"));
        assert_eq!(redacted.matches("[REDACTED]").count(), 8);
    }

    #[test]
    fn update_variants_are_explicit_and_do_not_mutate_canonical_source() {
        let root = std::env::temp_dir().join(format!("cooking-update-variant-{}", Uuid::new_v4()));
        let canonical = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/cooking/cooking-agent");
        let rollback = root.join("rollback");
        let abnormal = root.join("abnormal");
        let sequential = root.join("sequential");
        fs::create_dir_all(&root).expect("variant root");
        copy_source_tree(&canonical, &rollback).expect("rollback source");
        copy_source_tree(&canonical, &abnormal).expect("abnormal source");
        copy_source_tree(&canonical, &sequential).expect("sequential source");
        apply_update_variant(&rollback, UpdateVariant::Rollback).expect("rollback variant");
        apply_update_variant(&abnormal, UpdateVariant::Abnormal).expect("abnormal variant");
        apply_update_variant(&sequential, UpdateVariant::Rollback).expect("sequential rollback");
        apply_update_variant(&sequential, UpdateVariant::Abnormal).expect("sequential abnormal");

        let canonical_config =
            fs::read_to_string(canonical.join("agent.toml")).expect("canonical config");
        let rollback_config =
            fs::read_to_string(rollback.join("agent.toml")).expect("rollback config");
        let abnormal_config =
            fs::read_to_string(abnormal.join("agent.toml")).expect("abnormal config");
        let sequential_config =
            fs::read_to_string(sequential.join("agent.toml")).expect("sequential config");
        assert!(canonical_config.contains("arguments = [\"--migrate\"]"));
        assert!(rollback_config.contains("arguments = [\"--rollback-fixture\"]"));
        assert!(abnormal_config.contains("arguments = [\"--abnormal-fixture\"]"));
        assert!(sequential_config.contains("arguments = [\"--abnormal-fixture\"]"));
        assert!(
            fs::read_to_string(rollback.join("cooking_agent.py"))
                .expect("rollback source")
                .contains("deliberate migration rollback from v2")
        );
        let abnormal_source =
            fs::read_to_string(abnormal.join("cooking_agent.py")).expect("abnormal source");
        assert!(abnormal_source.contains("signal.SIGKILL"));
        let sequential_source =
            fs::read_to_string(sequential.join("cooking_agent.py")).expect("sequential source");
        assert!(sequential_source.contains("signal.SIGKILL"));
        assert_eq!(
            fs::read_to_string(canonical.join("agent.toml")).expect("canonical remains intact"),
            canonical_config
        );
        let _ = fs::remove_dir_all(root);
    }
}
