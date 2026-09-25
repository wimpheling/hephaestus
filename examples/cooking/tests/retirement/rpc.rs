use super::RetirementContext;
use connectrpc::client::{ClientConfig, HttpClient};
use rpc_proto::connect::hephaestus::{
    gateway::v1::GatewayServiceClient, instance::v1::AgentInstanceServiceClient,
    release::v1::ReleaseServiceClient, run::v1::RunServiceClient,
};
use rpc_proto::messages::hephaestus::common::v1::{OpaqueId, RequestContext};
use std::time::Duration;
use uuid::Uuid;

pub(crate) fn mutation_context(key: &str) -> RequestContext {
    RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: key.into(),
        ..Default::default()
    }
}

pub(crate) fn opaque(id: Uuid) -> OpaqueId {
    OpaqueId {
        value: id.to_string(),
        ..Default::default()
    }
}

pub(crate) fn config(
    ctx: &RetirementContext<'_>,
    audience: &str,
) -> Result<ClientConfig, Box<dyn std::error::Error + Send + Sync>> {
    Ok(
        ClientConfig::new(format!("http://{}", ctx.running.http_addr()).parse()?)
            .with_default_header(
                http::header::AUTHORIZATION,
                http::HeaderValue::from_str(&format!("Bearer {}", (ctx.token_factory)(audience)))?,
            )
            .with_default_timeout(Duration::from_secs(30)),
    )
}

pub(crate) fn instance_client(
    ctx: &RetirementContext<'_>,
    audience: &str,
) -> Result<AgentInstanceServiceClient<HttpClient>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(AgentInstanceServiceClient::new(
        HttpClient::plaintext(),
        config(ctx, audience)?,
    ))
}

pub(crate) fn gateway_client(
    ctx: &RetirementContext<'_>,
    audience: &str,
) -> Result<GatewayServiceClient<HttpClient>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(GatewayServiceClient::new(
        HttpClient::plaintext(),
        config(ctx, audience)?,
    ))
}

pub(crate) fn release_client(
    ctx: &RetirementContext<'_>,
    audience: &str,
) -> Result<ReleaseServiceClient<HttpClient>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(ReleaseServiceClient::new(
        HttpClient::plaintext(),
        config(ctx, audience)?,
    ))
}

pub(crate) fn run_client(
    ctx: &RetirementContext<'_>,
    audience: &str,
) -> Result<RunServiceClient<HttpClient>, Box<dyn std::error::Error + Send + Sync>> {
    Ok(RunServiceClient::new(
        HttpClient::plaintext(),
        config(ctx, audience)?,
    ))
}
