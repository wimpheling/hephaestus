use super::{
    AuthorityError, GatewayGoldenFixture, gateway_client, gateway_id, mutation_context, opaque,
};
use rpc_proto::messages::hephaestus::gateway::v1::{
    ConfigureGatewayRequest, CreateMailboxBindingRequest, GatewaySecretSelection,
};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn configure_rotated_inbound_gateway(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    previous: &GatewayGoldenFixture,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    inbound_import_id: Uuid,
    rotated_version_id: Uuid,
    mailbox_id: Uuid,
) -> Result<GatewayGoldenFixture, AuthorityError> {
    let gateway_id = gateway_id(pool, previous).await?;
    let expected_revision_id: Uuid =
        sqlx::query_scalar("SELECT active_revision_id FROM gateways WHERE id = $1")
            .bind(gateway_id)
            .fetch_one(pool)
            .await?;
    let old_version_id: Uuid = sqlx::query_scalar(
        "SELECT secret_version_id FROM gateway_secret_bindings
          WHERE gateway_id = $1 AND gateway_revision_id = $2 AND slot_key = 'webhook'",
    )
    .bind(gateway_id)
    .bind(expected_revision_id)
    .fetch_one(pool)
    .await?;
    let configure = gateway_client(
        running,
        token_factory,
        "/hephaestus.gateway.v1.GatewayService/ConfigureGateway",
    )?;
    let inbound_placeholder =
        crate::cooking_builds::cooking_inbound_placeholder(rotated_version_id);
    let response = configure
        .configure_gateway(ConfigureGatewayRequest {
            context: mutation_context("cooking-rotate-inbound-gateway", Uuid::new_v4()).into(),
            gateway_id: opaque(gateway_id).into(),
            expected_revision_id: opaque(expected_revision_id).into(),
            parameters: crate::cooking_builds::cooking_gateway_parameters(
                &inbound_placeholder,
                1001,
                1002,
            ),
            secret_selections: vec![GatewaySecretSelection {
                slot_key: String::from("webhook"),
                import_id: opaque(inbound_import_id).into(),
                secret_version_id: opaque(rotated_version_id).into(),
                route_path: String::from("/cooking/telegram"),
                header_name: String::from("x-telegram-bot-api-secret-token"),
                ..Default::default()
            }],
            ..Default::default()
        })
        .await?
        .into_owned();
    let revision_id = response
        .revision_id
        .into_option()
        .ok_or("rotated ConfigureGateway returned no revision")?
        .value
        .parse::<Uuid>()?;
    // Mailbox producer identity is also the deduplication scope and is
    // unique across immutable bindings for one mailbox. Scope this rotation
    // to its new revision so retained history cannot collide with it.
    let producer_id = format!("cooking-gateway-{revision_id}");
    let binding_client = gateway_client(
        running,
        token_factory,
        "/hephaestus.gateway.v1.GatewayService/CreateMailboxBinding",
    )?;
    let binding = binding_client
        .create_mailbox_binding(CreateMailboxBindingRequest {
            context: mutation_context("cooking-rotate-inbound-mailbox", Uuid::new_v4()).into(),
            gateway_revision_id: opaque(revision_id).into(),
            slot_key: String::from("cooking_requests"),
            mailbox_id: opaque(mailbox_id).into(),
            producer_id,
            ..Default::default()
        })
        .await?
        .into_owned();
    let binding = binding
        .binding
        .into_option()
        .ok_or("rotated CreateMailboxBinding returned no binding")?;
    let grant_id = binding
        .grant_id
        .into_option()
        .ok_or("rotated CreateMailboxBinding returned no grant")?
        .value
        .parse::<Uuid>()?;
    let retained_versions: Vec<Uuid> = sqlx::query_scalar(
        "SELECT secret_version_id FROM gateway_secret_bindings
          WHERE gateway_id = $1 AND slot_key = 'webhook'
          ORDER BY secret_version_id",
    )
    .bind(gateway_id)
    .fetch_all(pool)
    .await?;
    assert!(retained_versions.contains(&old_version_id));
    assert!(retained_versions.contains(&rotated_version_id));
    Ok(GatewayGoldenFixture {
        mailbox_id: previous.mailbox_id,
        grant_id,
    })
}

/// Confirms that gateway runtime leases retain both the old revision's
/// selected inbound version and the later rotated revision's version.
pub async fn assert_inbound_lease_history(
    pool: &PgPool,
    gateway: &GatewayGoldenFixture,
    old_version_id: Uuid,
    rotated_version_id: Uuid,
) -> Result<(), AuthorityError> {
    let gateway_id = gateway_id(pool, gateway).await?;
    let versions: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT lease.secret_version_id
           FROM gateway_secret_leases lease
           JOIN gateway_secret_bindings binding ON binding.id = lease.binding_id
          WHERE binding.gateway_id = $1",
    )
    .bind(gateway_id)
    .fetch_all(pool)
    .await?;
    assert!(
        versions.contains(&old_version_id),
        "old inbound lease is retained"
    );
    assert!(
        versions.contains(&rotated_version_id),
        "rotated inbound lease is retained"
    );
    assert_ne!(old_version_id, rotated_version_id);
    Ok(())
}
