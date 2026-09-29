use super::{RetirementContext, effect_counts, gateway_client, mutation_context, opaque};
use rpc_proto::messages::hephaestus::gateway::v1::{
    GatewayLifecycle, GetGatewayRequest, SetGatewayLifecycleRequest,
};

pub(crate) async fn retire_gateway(
    ctx: &RetirementContext<'_>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Remove the gateway through lifecycle CAS. `removed` is the supported
    // tombstone state; the edge's enabled-route query then yields an exact 404.
    let gateway_api = gateway_client(ctx, "/hephaestus.gateway.v1.GatewayService/GetGateway")?;
    let before_gateway = gateway_api
        .get_gateway(GetGatewayRequest {
            gateway_id: opaque(ctx.gateway_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let old_revision = before_gateway
        .gateway
        .as_option()
        .and_then(|g| g.active_revision_id.as_option())
        .ok_or("active gateway revision missing")?
        .value
        .clone();
    assert_eq!(
        before_gateway
            .gateway
            .as_option()
            .ok_or("gateway missing")?
            .lifecycle
            .to_i32(),
        GatewayLifecycle::Enabled as i32
    );

    let lifecycle_client = gateway_client(
        ctx,
        "/hephaestus.gateway.v1.GatewayService/SetGatewayLifecycle",
    )?;
    let transition = SetGatewayLifecycleRequest {
        context: mutation_context("cooking-retire-gateway").into(),
        gateway_id: opaque(ctx.gateway_id).into(),
        expected: GatewayLifecycle::Enabled.into(),
        next: GatewayLifecycle::Removed.into(),
        ..Default::default()
    };
    let changed = lifecycle_client
        .set_gateway_lifecycle(transition.clone())
        .await?
        .into_owned();
    assert_eq!(
        changed
            .gateway
            .as_option()
            .ok_or("lifecycle response missing")?
            .lifecycle
            .to_i32(),
        GatewayLifecycle::Removed as i32
    );
    assert!(changed.receipt.is_set());
    // Lifecycle mutations are compare-and-swap operations: replaying the
    // original Enabled expectation after removal is a stale precondition.
    let replay_error = lifecycle_client
        .set_gateway_lifecycle(transition)
        .await
        .expect_err("replayed removed gateway CAS unexpectedly succeeded");
    assert_eq!(
        replay_error.code,
        connectrpc::error::ErrorCode::FailedPrecondition
    );
    assert_eq!(
        replay_error.message.as_deref(),
        Some("operation precondition failed")
    );

    let gateway_history = gateway_client(ctx, "/hephaestus.gateway.v1.GatewayService/GetGateway")?
        .get_gateway(GetGatewayRequest {
            gateway_id: opaque(ctx.gateway_id).into(),
            ..Default::default()
        })
        .await?
        .into_owned();
    let gateway = gateway_history
        .gateway
        .as_option()
        .ok_or("removed gateway history missing")?;
    assert_eq!(gateway.lifecycle.to_i32(), GatewayLifecycle::Removed as i32);
    assert_eq!(
        gateway
            .active_revision_id
            .as_option()
            .ok_or("active revision history missing")?
            .value,
        old_revision
    );
    assert!(!gateway_history.revisions.is_empty());

    let before_effects = effect_counts(ctx).await?;
    let response = super::super::cooking::caddy_gateway_client()
        .post(format!("{}/gateway/cooking/telegram", ctx.public_url.trim_end_matches('/')))
        .header("x-telegram-bot-api-secret-token", ctx.valid_inbound_credential)
        .json(&serde_json::json!({"update_id": 9002, "message": {"from": {"id": 1001}, "text": "retired-gateway"}}))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    response.bytes().await?;
    assert_eq!(effect_counts(ctx).await?, before_effects);
    Ok(())
}
