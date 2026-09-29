use super::{
    AuthorityError, GatewayGoldenFixture, gateway_client, gateway_client_with_token, gateway_id,
    mutation_context, opaque, outsider_token,
};
use connectrpc::error::ErrorCode;
use identity_domain::{BrowserSessionSid, UserId};
use rpc_proto::messages::hephaestus::{
    common::v1::PageRequest,
    gateway::v1::{
        GetGatewayRequest, ListMailboxPublicationsRequest, RevokeMailboxBindingGrantRequest,
    },
};
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

pub async fn retire_cooking_gateway_grant(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    gateway: &GatewayGoldenFixture,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    inbound_credential: &str,
    outsider_id: UserId,
    outsider_browser_session: BrowserSessionSid,
) -> Result<(), AuthorityError> {
    let active_binding: (Uuid, Uuid) = sqlx::query_as(
        "SELECT binding.id, binding_grant.id
           FROM gateway_mailbox_bindings binding
           JOIN gateway_mailbox_binding_grants binding_grant ON binding_grant.binding_id = binding.id
           JOIN gateways gateway ON gateway.id = binding.gateway_id
          WHERE binding.mailbox_id = $1 AND binding_grant.id = $2
            AND binding_grant.status = 'active'
            AND gateway.active_revision_id = binding.gateway_revision_id",
    )
    .bind(gateway.mailbox_id.as_uuid())
    .bind(gateway.grant_id)
    .fetch_one(pool)
    .await?;
    assert_eq!(active_binding.1, gateway.grant_id);

    let audience = "/hephaestus.gateway.v1.GatewayService/RevokeMailboxBindingGrant";
    let client = gateway_client(running, token_factory, audience)?;
    let revoke_key = "cooking-authority-revoke-gateway-mailbox";
    let revoked = client
        .revoke_mailbox_binding_grant(RevokeMailboxBindingGrantRequest {
            context: mutation_context(revoke_key, Uuid::new_v4()).into(),
            binding_id: opaque(active_binding.0).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let revoked_binding = revoked
        .binding
        .as_option()
        .ok_or("revoke returned no binding")?;
    let revoke_receipt = revoked
        .receipt
        .as_option()
        .ok_or("revoke returned no receipt")?;
    assert_eq!(
        revoked_binding
            .grant_id
            .as_option()
            .map(|id| id.value.clone()),
        Some(active_binding.1.to_string())
    );

    // Revoke itself is idempotent across transport retries.
    let replay_context = mutation_context(revoke_key, Uuid::new_v4());
    let replay = client
        .revoke_mailbox_binding_grant(RevokeMailboxBindingGrantRequest {
            context: replay_context.clone().into(),
            binding_id: opaque(active_binding.0).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    assert_eq!(replay.receipt.as_option(), Some(revoke_receipt));
    assert_eq!(
        replay
            .binding
            .as_option()
            .map(|binding| binding.grant_status.as_str()),
        Some("revoked")
    );

    // CreateMailboxBinding's exact receipt and replay are proved by the
    // build helper before browser reconfiguration. Retiring the current
    // browser-selected binding must not reuse that earlier revision key.
    let status: String =
        sqlx::query_scalar("SELECT status FROM gateway_mailbox_binding_grants WHERE id = $1")
            .bind(active_binding.1)
            .fetch_one(pool)
            .await?;
    assert_eq!(status, "revoked");

    assert_revoked_gateway_history(
        pool,
        running,
        gateway,
        token_factory,
        outsider_id,
        outsider_browser_session,
    )
    .await?;
    assert_revoked_ingress(pool, gateway, inbound_credential).await?;
    Ok(())
}

pub(crate) async fn assert_revoked_gateway_history(
    pool: &PgPool,
    running: &hephaestus_app::RunningHephaestus,
    gateway: &GatewayGoldenFixture,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    outsider_id: UserId,
    outsider_browser_session: BrowserSessionSid,
) -> Result<(), AuthorityError> {
    let browser_gateway_id = gateway_id(pool, gateway).await?;
    let current = gateway_client(
        running,
        token_factory,
        "/hephaestus.gateway.v1.GatewayService/GetGateway",
    )?
    .get_gateway(GetGatewayRequest {
        gateway_id: opaque(browser_gateway_id).into(),
        ..Default::default()
    })
    .await?
    .into_owned();
    assert!(current.gateway.as_option().is_some());

    let history = gateway_client(
        running,
        token_factory,
        "/hephaestus.gateway.v1.GatewayService/ListMailboxPublications",
    )?
    .list_mailbox_publications(ListMailboxPublicationsRequest {
        gateway_id: opaque(browser_gateway_id).into(),
        page: PageRequest {
            page_size: 64,
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
    .await?
    .into_owned();
    assert!(
        history
            .publications
            .iter()
            .any(|publication| publication.run_id.is_set()),
        "prior publication/run history must remain inspectable"
    );

    let outsider = gateway_client_with_token(
        running,
        &outsider_token(
            "/hephaestus.gateway.v1.GatewayService/GetGateway",
            outsider_id,
            outsider_browser_session,
        ),
    )?;
    let outsider_error = outsider
        .get_gateway(GetGatewayRequest {
            gateway_id: opaque(browser_gateway_id).into(),
            ..Default::default()
        })
        .await
        .expect_err("an outsider must not inspect the cooking gateway");
    // Gateway reads check CanRead before loading the resource, so an
    // authenticated outsider receives the generic authorization denial.
    assert_eq!(outsider_error.code, ErrorCode::PermissionDenied);

    let outsider_history = gateway_client_with_token(
        running,
        &outsider_token(
            "/hephaestus.gateway.v1.GatewayService/ListMailboxPublications",
            outsider_id,
            outsider_browser_session,
        ),
    )?;
    let outsider_history_error = outsider_history
        .list_mailbox_publications(ListMailboxPublicationsRequest {
            gateway_id: opaque(browser_gateway_id).into(),
            page: PageRequest {
                page_size: 64,
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .await
        .expect_err("an outsider must not inspect gateway publication history");
    assert_eq!(outsider_history_error.code, ErrorCode::PermissionDenied);
    Ok(())
}

pub(crate) async fn assert_revoked_ingress(
    pool: &PgPool,
    gateway: &GatewayGoldenFixture,
    inbound_credential: &str,
) -> Result<(), AuthorityError> {
    let before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
            .bind(gateway.mailbox_id.as_uuid())
            .fetch_one(pool)
            .await?;
    let before_deliveries: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_deliveries WHERE mailbox_id = $1")
            .bind(gateway.mailbox_id.as_uuid())
            .fetch_one(pool)
            .await?;
    let before_runs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM runs WHERE instance_id = (
        SELECT instance_id FROM mailboxes WHERE id = $1
    )",
    )
    .bind(gateway.mailbox_id.as_uuid())
    .fetch_one(pool)
    .await?;
    let public = std::env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")?;
    let response =
        super::super::cooking::caddy_gateway_client_with_timeout(Duration::from_secs(30))
            .post(format!("{public}/gateway/cooking/telegram"))
            .header("x-telegram-bot-api-secret-token", inbound_credential)
            .json(&serde_json::json!({
                "update_id": 9001,
                "message": {"from": {"id": 1001}, "text": "retired"}
            }))
            .send()
            .await?;
    assert_eq!(response.status(), reqwest::StatusCode::BAD_GATEWAY);
    response.bytes().await?;
    let after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
            .bind(gateway.mailbox_id.as_uuid())
            .fetch_one(pool)
            .await?;
    assert_eq!(after, before);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM mailbox_deliveries WHERE mailbox_id = $1",
        )
        .bind(gateway.mailbox_id.as_uuid())
        .fetch_one(pool)
        .await?,
        before_deliveries,
        "revoked ingress must not create a delivery attempt"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM runs WHERE instance_id = (
            SELECT instance_id FROM mailboxes WHERE id = $1
        )"
        )
        .bind(gateway.mailbox_id.as_uuid())
        .fetch_one(pool)
        .await?,
        before_runs,
        "revoked ingress must not create a run"
    );
    Ok(())
}
