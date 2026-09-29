use forge_domain::{OrganizationId, ProjectId};
use hephaestus_app::RunningHephaestus;
use identity_domain::AuthenticatedIdentity;
use rpc_proto::messages::hephaestus::{
    common::v1::{NetworkPolicy, ParameterValue, RuntimePolicy, parameter_value::Value},
    instance::v1::{
        CapabilityBindingSelection, CreateAttachmentRequest, ImportAgentRequest, RefSelector,
        ReviseCapabilitiesRequest, TriggerPolicy, ref_selector,
    },
    release::v1::{
        InstallUiRequest, UiInstallationLifecycle, UiInstallationTarget, ui_installation_target,
    },
};
use sqlx::PgPool;
use uuid::Uuid;

use super::{
    helpers::release_client,
    state::{ForkTargetProvisioningState, ForkTargetState},
};

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
    let imported = super::super::instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/ImportAgent",
    )
    .expect("fork target ImportAgent client")
    .import_agent(ImportAgentRequest {
        context: super::super::mutation_context("fork-target-import").into(),
        project_id: super::super::opaque(project.as_uuid()).into(),
        release_agent_id: super::super::opaque(release_agent_id).into(),
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
        super::super::response_id(imported.instance_id.into_option(), "fork target instance")
            .expect("fork target instance ID");
    let revision_id =
        super::super::response_id(imported.revision_id.into_option(), "fork target revision")
            .expect("fork target revision ID");
    assert_ne!(instance_id, target.source_instance_id);
    assert_ne!(revision_id, target.source_revision_id);

    let attachment = super::super::instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/CreateAttachment",
    )
    .expect("fork target CreateAttachment client")
    .create_attachment(CreateAttachmentRequest {
        context: super::super::mutation_context("fork-target-attachment").into(),
        instance_id: super::super::opaque(instance_id).into(),
        repository_id: super::super::opaque(target.repository_id).into(),
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
    let attachment_id = super::super::response_id(
        attachment.attachment_id.into_option(),
        "fork target attachment",
    )
    .expect("fork target attachment ID");
    assert_ne!(attachment_id, target.source_attachment_id);

    let revised = super::super::instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/ReviseCapabilities",
    )
    .expect("fork target ReviseCapabilities client")
    .revise_capabilities(ReviseCapabilitiesRequest {
        context: super::super::mutation_context("fork-target-capabilities").into(),
        instance_id: super::super::opaque(instance_id).into(),
        expected_revision_id: super::super::opaque(revision_id).into(),
        bindings: vec![CapabilityBindingSelection {
            slot_key: String::from("session"),
            resource_kind: String::from("repository"),
            resource_id: super::super::opaque(target.repository_id).into(),
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
    let capability_revision_id = super::super::response_id(
        revised.instance_revision_id.into_option(),
        "fork target capability revision",
    )
    .expect("fork target capability revision ID");
    assert_ne!(capability_revision_id, target.source_revision_id);
    let fork_model_alias = format!("model_fork_{}", model_rule_id.simple());
    let (bound_revision_id, binding_id) = super::super::seed_model_secret_with_rule(
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
            context: super::super::mutation_context("fork-target-ui").into(),
            organization_id: super::super::opaque(organization.as_uuid()).into(),
            target: UiInstallationTarget {
                target: Some(ui_installation_target::Target::RepositoryId(
                    super::super::opaque(target.repository_id).into(),
                )),
                ..Default::default()
            }
            .into(),
            release_id: super::super::opaque(release_id).into(),
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
    let installation_id = super::super::response_id(
        installed.installation_id.into_option(),
        "fork target UI installation",
    )
    .expect("fork target UI installation ID");
    let generation_id = super::super::response_id(
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
