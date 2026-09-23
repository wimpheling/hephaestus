//! Fork-phase production Git evidence for the session-chat composed test.
//!
//! The composed browser acceptance uses this module for production fork
//! publication and fresh target provisioning.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use buffa_types::google::protobuf::Timestamp;
use forge_domain::{OrganizationId, ProjectId};
use hephaestus_app::RunningHephaestus;
use identity_domain::AuthenticatedIdentity;
use rpc_proto::{
    connect::hephaestus::{
        pat::v1::PersonalAccessTokenServiceClient, release::v1::ReleaseServiceClient,
        repository::v1::RepositoryServiceClient,
    },
    messages::hephaestus::repository::v1::CreateRepositoryRequest,
    messages::hephaestus::{
        common::v1::{NetworkPolicy, ParameterValue, RuntimePolicy, parameter_value::Value},
        instance::v1::{
            CapabilityBindingSelection, CreateAttachmentRequest, ImportAgentRequest, RefSelector,
            ReviseCapabilitiesRequest, TriggerPolicy, ref_selector,
        },
        pat::v1::{CreatePersonalAccessTokenRequest, GitOperation, PersonalAccessTokenScope},
        release::v1::{
            InstallUiRequest, UiInstallationLifecycle, UiInstallationTarget, ui_installation_target,
        },
    },
};
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use time::OffsetDateTime;
use tokio::process::Command;
use uuid::Uuid;

/// Durable source facts captured immediately before the fork begins.
#[derive(Debug, Clone)]
pub struct SourceSessionState {
    pub repository_id: Uuid,
    pub session_id: Uuid,
    pub release_id: Uuid,
    pub release_agent_id: Uuid,
    pub model_rule_id: Uuid,
    pub instance_id: Uuid,
    pub revision_id: Uuid,
    pub attachment_id: Uuid,
    pub installation_id: Uuid,
    pub generation_id: Uuid,
    pub model_binding_id: Uuid,
    pub head: String,
    pub reachable_objects: BTreeSet<String>,
    pub record_blobs: BTreeMap<String, Vec<u8>>,
    pub accepted_receive_count: i64,
}

/// Target facts retained for the later fresh-instance/browser phase.
#[derive(Debug, Clone)]
pub struct ForkTargetState {
    pub source_repository_id: Uuid,
    pub source_head: String,
    pub source_accepted_receive_count: i64,
    pub source_release_id: Uuid,
    pub source_release_agent_id: Uuid,
    pub source_model_rule_id: Uuid,
    pub source_model_binding_id: Uuid,
    pub source_instance_id: Uuid,
    pub source_revision_id: Uuid,
    pub source_attachment_id: Uuid,
    pub source_installation_id: Uuid,
    pub source_generation_id: Uuid,
    pub repository_id: Uuid,
    pub checkout: PathBuf,
    pub session_id: Uuid,
    pub manifest_commit: String,
    pub manifest_path: String,
    pub initial_accepted_receive_count: i64,
}

/// Fresh target platform objects created after the fork is published.
#[allow(clippy::struct_field_names)] // Persisted platform IDs stay explicit at this test boundary.
#[derive(Debug, Clone, Copy)]
pub struct ForkTargetProvisioningState {
    pub instance_id: Uuid,
    pub revision_id: Uuid,
    pub attachment_id: Uuid,
    pub capability_revision_id: Uuid,
    pub bound_revision_id: Uuid,
    pub binding_id: Uuid,
    pub installation_id: Uuid,
    pub generation_id: Uuid,
}

/// Publishes a fork through the production Git HTTP boundary and proves that
/// the source history is retained without mutating the source repository.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)] // This test keeps the publication proof in one auditable production boundary.
pub async fn exercise(
    pool: &PgPool,
    running: &RunningHephaestus,
    root: &Path,
    source_root: &Path,
    project: ProjectId,
    source: SourceSessionState,
    git_token: &str,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
) -> ForkTargetState {
    let actual_source_head = super::git_output_bare(
        root,
        source.repository_id,
        &["rev-parse", "refs/heads/main"],
    )
    .await;
    assert_eq!(
        actual_source_head, source.head,
        "source snapshot head must match the live source ref before forking"
    );
    let actual_source_objects = object_ids(root, source.repository_id, &source.head).await;
    assert!(
        !actual_source_objects.is_empty(),
        "source snapshot must contain reachable objects"
    );
    assert_eq!(
        actual_source_objects, source.reachable_objects,
        "source snapshot must still describe the live source history"
    );
    assert!(
        !commit_ids(root, source.repository_id, &source.head)
            .await
            .is_empty(),
        "source snapshot must contain reachable commits"
    );
    assert_eq!(
        super::accepted_receive_count(pool, source.repository_id).await,
        source.accepted_receive_count,
        "source receive snapshot must still be current before forking"
    );
    let target_repository_id = create_target_repository(running, rpc_token, project).await;
    let target_pat = issue_target_pat(running, rpc_token, target_repository_id).await;

    let source_clone = root.join(format!("session-chat-fork-source-{}", Uuid::new_v4()));
    let target_checkout = root.join(format!("session-chat-fork-target-{}", Uuid::new_v4()));
    let source_remote = format!("http://{}/{}", running.http_addr(), source.repository_id);
    let source_clone_arg = source_clone.to_string_lossy().into_owned();
    let clone_arguments = [
        "clone",
        "--branch",
        "main",
        source_remote.as_str(),
        source_clone_arg.as_str(),
    ];
    super::authenticated_git(root, git_token, &clone_arguments).await;
    assert_eq!(
        super::git_output(&source_clone, &["rev-parse", "refs/heads/main"]).await,
        source.head,
        "HTTP source clone must start at the captured source head"
    );

    let target_session_id = Uuid::new_v4();
    fork_local_checkout(
        source_root,
        &source_clone,
        &target_checkout,
        target_session_id,
    )
    .await;
    let target_remote = format!("http://{}/{}", running.http_addr(), target_repository_id);
    let add_remote_arguments = ["remote", "add", "origin", target_remote.as_str()];
    super::git(&target_checkout, &add_remote_arguments).await;
    let push_arguments = ["push", "origin", "HEAD:refs/heads/main"];
    authenticated_git_pat(&target_checkout, &target_pat, &push_arguments).await;

    let manifest_commit = super::git_output_bare(
        root,
        target_repository_id,
        &["rev-parse", "refs/heads/main"],
    )
    .await;
    let source_commit_ids = commit_ids(root, source.repository_id, &source.head).await;
    let target_commit_ids = commit_ids(root, target_repository_id, &manifest_commit).await;
    assert!(
        source_commit_ids.is_subset(&target_commit_ids),
        "fork target must contain every source reachable commit"
    );
    assert_eq!(
        target_commit_ids.difference(&source_commit_ids).count(),
        1,
        "fork publication must add exactly one commit before the target turn"
    );
    assert_eq!(
        super::git_output_bare(
            root,
            target_repository_id,
            &["rev-parse", &format!("{manifest_commit}^")],
        )
        .await,
        source.head,
        "fork manifest must directly descend from the source head"
    );

    let target_objects = object_ids(root, target_repository_id, &manifest_commit).await;
    assert!(
        source
            .reachable_objects
            .iter()
            .all(|object| target_objects.contains(object)),
        "fork target must retain every source reachable object"
    );
    for (path, expected) in &source.record_blobs {
        assert_eq!(
            super::git_output_bare_bytes(
                root,
                target_repository_id,
                &["show", &format!("{manifest_commit}:{path}")],
            )
            .await,
            expected.as_slice(),
            "fork target must retain the source record bytes: {path}"
        );
    }
    let manifest_paths = super::git_output_bare(
        root,
        target_repository_id,
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "-r",
            &manifest_commit,
        ],
    )
    .await
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(
        manifest_paths.len(),
        1,
        "fork commit must contain only its new session manifest"
    );
    let manifest_path = manifest_paths
        .into_iter()
        .next()
        .expect("fork manifest path");
    assert_eq!(
        manifest_path, ".heph/session/v1/manifest.json",
        "fork commit must update the release-owned session manifest"
    );
    let manifest: JsonValue = serde_json::from_str(
        &super::git_output_bare(
            root,
            target_repository_id,
            &["show", &format!("{manifest_commit}:{manifest_path}")],
        )
        .await,
    )
    .expect("fork manifest JSON");
    assert_eq!(manifest["kind"], "session_manifest");
    assert_eq!(
        manifest["data"]["session_id"],
        target_session_id.to_string()
    );
    assert_eq!(
        manifest["data"]["forked_from_session_id"],
        source.session_id.to_string()
    );

    assert_eq!(
        super::git_output(&target_checkout, &["remote", "get-url", "origin"]).await,
        target_remote,
        "fork checkout must point only at the target repository"
    );
    assert_eq!(
        super::git_output_bare(
            root,
            source.repository_id,
            &["rev-parse", "refs/heads/main"]
        )
        .await,
        source.head,
        "fork must preserve the source main ref"
    );
    assert_eq!(
        super::accepted_receive_count(pool, source.repository_id).await,
        source.accepted_receive_count,
        "fork must not add a source receive"
    );

    ForkTargetState {
        source_repository_id: source.repository_id,
        source_head: source.head,
        source_accepted_receive_count: source.accepted_receive_count,
        source_release_id: source.release_id,
        source_release_agent_id: source.release_agent_id,
        source_model_rule_id: source.model_rule_id,
        source_model_binding_id: source.model_binding_id,
        source_instance_id: source.instance_id,
        source_revision_id: source.revision_id,
        source_attachment_id: source.attachment_id,
        source_installation_id: source.installation_id,
        source_generation_id: source.generation_id,
        repository_id: target_repository_id,
        checkout: target_checkout,
        session_id: target_session_id,
        manifest_commit,
        manifest_path,
        initial_accepted_receive_count: super::accepted_receive_count(pool, target_repository_id)
            .await,
    }
}

/// Provisions fresh target authority and the repository-scoped session UI.
///
/// The caller supplies the release identity and a fresh model rule ID; no
/// source instance, revision, attachment, rule, or secret binding is reused.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // The target setup is one bounded production provisioning proof.
pub async fn provision_target(
    pool: &PgPool,
    running: &RunningHephaestus,
    project: ProjectId,
    organization: OrganizationId,
    identity: &AuthenticatedIdentity,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    target: &ForkTargetState,
    release_id: Uuid,
    release_agent_id: Uuid,
    model_rule_id: Uuid,
) -> ForkTargetProvisioningState {
    assert_eq!(
        release_id, target.source_release_id,
        "target release must be the source release selected for the fork"
    );
    assert_eq!(
        release_agent_id, target.source_release_agent_id,
        "target agent must be the source release agent selected for the fork"
    );
    assert_ne!(
        model_rule_id, target.source_model_rule_id,
        "fork target must use a fresh model rule ID"
    );
    let imported = super::instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/ImportAgent",
    )
    .expect("fork target ImportAgent client")
    .import_agent(ImportAgentRequest {
        context: super::mutation_context("fork-target-import").into(),
        project_id: super::opaque(project.as_uuid()).into(),
        release_agent_id: super::opaque(release_agent_id).into(),
        name: String::from("reference-session-chat-fork-target"),
        parameters: vec![ParameterValue {
            name: String::from("model_rule_id"),
            value: Some(Value::StringValue(model_rule_id.to_string())),
            ..Default::default()
        }],
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
    .expect("ImportAgent fork target release")
    .into_owned();
    let instance_id =
        super::response_id(imported.instance_id.into_option(), "fork target instance")
            .expect("fork target instance ID");
    let revision_id =
        super::response_id(imported.revision_id.into_option(), "fork target revision")
            .expect("fork target revision ID");
    assert_ne!(instance_id, target.source_instance_id);
    assert_ne!(revision_id, target.source_revision_id);

    let attachment = super::instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/CreateAttachment",
    )
    .expect("fork target CreateAttachment client")
    .create_attachment(CreateAttachmentRequest {
        context: super::mutation_context("fork-target-attachment").into(),
        instance_id: super::opaque(instance_id).into(),
        repository_id: super::opaque(target.repository_id).into(),
        ref_selector: RefSelector {
            selector: Some(ref_selector::Selector::Exact(String::from(
                "refs/heads/main",
            ))),
            ..Default::default()
        }
        .into(),
        trigger_policy: TriggerPolicy::Push.into(),
        ..Default::default()
    })
    .await
    .expect("CreateAttachment fork target repository")
    .into_owned();
    let attachment_id = super::response_id(
        attachment.attachment_id.into_option(),
        "fork target attachment",
    )
    .expect("fork target attachment ID");
    assert_ne!(attachment_id, target.source_attachment_id);

    let revised = super::instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/ReviseCapabilities",
    )
    .expect("fork target ReviseCapabilities client")
    .revise_capabilities(ReviseCapabilitiesRequest {
        context: super::mutation_context("fork-target-capabilities").into(),
        instance_id: super::opaque(instance_id).into(),
        expected_revision_id: super::opaque(revision_id).into(),
        bindings: vec![CapabilityBindingSelection {
            slot_key: String::from("session"),
            resource_kind: String::from("repository"),
            resource_id: super::opaque(target.repository_id).into(),
            granted_operations: vec![String::from("git_read"), String::from("update_ref")],
            ..Default::default()
        }],
        ..Default::default()
    })
    .await
    .expect("revise fork target capability")
    .into_owned();
    assert!(
        !revised.runnable,
        "fork target must remain gated until its fresh model binding is installed"
    );
    let capability_revision_id = super::response_id(
        revised.instance_revision_id.into_option(),
        "fork target capability revision",
    )
    .expect("fork target capability revision ID");
    assert_ne!(capability_revision_id, target.source_revision_id);
    let fork_model_alias = format!("model_fork_{}", model_rule_id.simple());
    let (bound_revision_id, binding_id) = super::seed_model_secret_with_rule(
        pool,
        organization,
        project,
        identity,
        instance_id,
        capability_revision_id,
        attachment_id,
        model_rule_id,
        &fork_model_alias,
    )
    .await;
    assert_ne!(bound_revision_id, target.source_revision_id);
    assert_ne!(bound_revision_id, capability_revision_id);

    let release_client = release_client(
        running,
        rpc_token,
        "/hephaestus.release.v1.ReleaseService/InstallUi",
    );
    let installed = release_client
        .install_ui(InstallUiRequest {
            context: super::mutation_context("fork-target-ui").into(),
            organization_id: super::opaque(organization.as_uuid()).into(),
            target: UiInstallationTarget {
                target: Some(ui_installation_target::Target::RepositoryId(
                    super::opaque(target.repository_id).into(),
                )),
                ..Default::default()
            }
            .into(),
            release_id: super::opaque(release_id).into(),
            ui_key: String::from("session-chat"),
            acknowledge_repository_git_access: true,
            ..Default::default()
        })
        .await
        .expect("install fork target session UI")
        .into_owned();
    assert_eq!(
        installed.lifecycle.to_i32(),
        UiInstallationLifecycle::Enabled as i32,
        "fork target session UI must be enabled"
    );
    let installation_id = super::response_id(
        installed.installation_id.into_option(),
        "fork target UI installation",
    )
    .expect("fork target UI installation ID");
    let generation_id = super::response_id(
        installed.generation_id.into_option(),
        "fork target UI generation",
    )
    .expect("fork target UI generation ID");
    assert_ne!(installation_id, target.source_installation_id);
    assert_ne!(generation_id, target.source_generation_id);

    ForkTargetProvisioningState {
        instance_id,
        revision_id,
        attachment_id,
        capability_revision_id,
        bound_revision_id,
        binding_id,
        installation_id,
        generation_id,
    }
}

async fn create_target_repository(
    running: &RunningHephaestus,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    project: ProjectId,
) -> Uuid {
    let audience = "/hephaestus.repository.v1.RepositoryService/CreateRepository";
    let uri = format!("http://{}", running.http_addr())
        .parse()
        .expect("repository RPC endpoint URI");
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", rpc_token(audience)))
                .expect("repository RPC authorization header"),
        )
        .with_default_timeout(Duration::from_secs(30));
    let client = RepositoryServiceClient::new(connectrpc::client::HttpClient::plaintext(), config);
    let response = client
        .create_repository(CreateRepositoryRequest {
            context: super::mutation_context("fork-target-repository").into(),
            project_id: super::opaque(project.as_uuid()).into(),
            name: format!("session-chat-fork-{}", Uuid::new_v4()),
            default_branch: String::from("main"),
            is_public: false,
            agent_runs_enabled: true,
            ..Default::default()
        })
        .await
        .expect("create fork target repository through RPC")
        .into_owned();
    super::response_id(
        response.repository_id.into_option(),
        "fork target repository",
    )
    .expect("fork target repository ID")
}

fn release_client(
    running: &RunningHephaestus,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> ReleaseServiceClient<connectrpc::client::HttpClient> {
    let uri = format!("http://{}", running.http_addr())
        .parse()
        .expect("release RPC endpoint URI");
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", rpc_token(audience)))
                .expect("release RPC authorization header"),
        )
        .with_default_timeout(Duration::from_secs(30));
    ReleaseServiceClient::new(connectrpc::client::HttpClient::plaintext(), config)
}

async fn issue_target_pat(
    running: &RunningHephaestus,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    repository_id: Uuid,
) -> String {
    let audience = "/hephaestus.pat.v1.PersonalAccessTokenService/CreatePersonalAccessToken";
    let uri = format!("http://{}", running.http_addr())
        .parse()
        .expect("PAT RPC endpoint URI");
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", rpc_token(audience)))
                .expect("PAT RPC authorization header"),
        )
        .with_default_timeout(Duration::from_secs(30));
    let client =
        PersonalAccessTokenServiceClient::new(connectrpc::client::HttpClient::plaintext(), config);
    let expires_at = OffsetDateTime::now_utc() + time::Duration::hours(1);
    let response = client
        .create_personal_access_token(CreatePersonalAccessTokenRequest {
            context: super::mutation_context("fork-target-pat").into(),
            label: format!("session-chat-fork-target-{repository_id}"),
            scope: PersonalAccessTokenScope {
                operations: vec![
                    GitOperation::Discover.into(),
                    GitOperation::Fetch.into(),
                    GitOperation::Receive.into(),
                ],
                repository_ids: vec![super::opaque(repository_id)],
                ..Default::default()
            }
            .into(),
            expires_at: Timestamp {
                seconds: expires_at.unix_timestamp(),
                nanos: i32::try_from(expires_at.nanosecond()).expect("PAT expiry nanos"),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await
        .expect("issue target-scoped Git PAT")
        .into_owned();
    let metadata = response.token.into_option().expect("target PAT metadata");
    let scope = metadata.scope.into_option().expect("target PAT scope");
    assert_eq!(scope.repository_ids, vec![super::opaque(repository_id)]);
    assert_eq!(
        scope
            .operations
            .iter()
            .map(buffa::enumeration::EnumValue::as_known)
            .collect::<Option<Vec<_>>>(),
        Some(vec![
            GitOperation::Discover,
            GitOperation::Fetch,
            GitOperation::Receive
        ])
    );
    let value = response
        .value
        .into_option()
        .expect("target PAT value")
        .value;
    String::from_utf8(value).unwrap_or_else(|_| panic!("target PAT response was not UTF-8"))
}

async fn fork_local_checkout(
    source_root: &Path,
    source_clone: &Path,
    target_checkout: &Path,
    target_session_id: Uuid,
) {
    let script = r"
from pathlib import Path
import sys
from git_adapter import LocalGitSession

source = LocalGitSession.open(Path(sys.argv[1]))
source.fork(Path(sys.argv[2]), sys.argv[3])
";
    let output = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(source_clone)
        .arg(target_checkout)
        .arg(target_session_id.to_string())
        .env("PYTHONPATH", source_root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .await
        .expect("run LocalGitSession fork helper");
    assert!(
        output.status.success(),
        "LocalGitSession fork helper failed"
    );
}

async fn authenticated_git_pat(directory: &Path, token: &str, arguments: &[&str]) {
    let basic = BASE64_STANDARD.encode(format!("heph-pat:{token}"));
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Basic {basic}"),
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .await
        .expect("run target PAT Git command");
    assert!(output.status.success(), "target PAT Git command failed");
}

async fn commit_ids(root: &Path, repository_id: Uuid, head: &str) -> BTreeSet<String> {
    super::git_output_bare(root, repository_id, &["rev-list", head])
        .await
        .lines()
        .map(str::to_owned)
        .collect()
}

async fn object_ids(root: &Path, repository_id: Uuid, head: &str) -> BTreeSet<String> {
    super::git_output_bare(root, repository_id, &["rev-list", "--objects", head])
        .await
        .lines()
        .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
        .collect()
}
