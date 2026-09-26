// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
/// Imports the published cooking agent and attaches a separately created blog
/// repository through the production instance RPCs.
pub(crate) async fn prepare_cooking_instance(
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
pub(crate) async fn prepare_cooking_instance_variant(
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
pub(crate) async fn install_cooking_gateway(
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
        current.gateway.into_option().and_then(|summary| {
            // HTTP services are installed with a desired revision before the
            // asynchronous reconciler makes it active. Stateless gateways
            // have no desired service revision, so retain their active one.
            summary
                .desired_service_revision_id
                .into_option()
                .or_else(|| summary.active_revision_id.into_option())
        }),
        "GetGateway declared revision",
    )?;
    Ok(InstalledCookingGateway {
        gateway_id,
        revision_id,
    })
}
