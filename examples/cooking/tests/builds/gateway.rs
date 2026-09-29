// Reuse the parent facade so sibling fixture phases share one boundary context.
#[allow(unused_imports)]
use super::*;
/// Configures the installed gateway and binds its declared publication slot
/// to the mailbox created by [`prepare_cooking_instance`].
#[allow(clippy::too_many_lines)] // Keep the RPC receipt/replay proof in order.
pub(crate) async fn configure_cooking_gateway(
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
