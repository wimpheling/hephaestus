use super::{AuthorityError, GatewayGoldenFixture};
use connectrpc::client::ClientConfig;
use identity_domain::{BrowserSessionSid, UserId};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rpc_proto::{
    connect::hephaestus::gateway::v1::GatewayServiceClient,
    messages::hephaestus::common::v1::{OpaqueId, RequestContext},
};
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

pub(crate) fn gateway_client(
    running: &hephaestus_app::RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<GatewayServiceClient<connectrpc::client::HttpClient>, AuthorityError> {
    gateway_client_with_token(running, &token_factory(audience))
}

pub(crate) fn gateway_client_with_token(
    running: &hephaestus_app::RunningHephaestus,
    token: &str,
) -> Result<GatewayServiceClient<connectrpc::client::HttpClient>, AuthorityError> {
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {token}"))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok(GatewayServiceClient::new(
        connectrpc::client::HttpClient::plaintext(),
        config,
    ))
}

pub(crate) fn outsider_token(
    audience: &str,
    outsider_id: UserId,
    outsider_browser_session: BrowserSessionSid,
) -> String {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": "hephaestus-web-mediator",
            "sub": outsider_id.to_string(),
            "aud": audience,
            "iat": now,
            "nbf": now,
            "exp": now + 25,
            "jti": Uuid::new_v4().to_string(),
            "sid": outsider_browser_session.to_protocol_string()
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        )),
    )
    .expect("sign outsider mediator token")
}

pub(crate) fn mutation_context(key: &str, request_id: Uuid) -> RequestContext {
    RequestContext {
        request_id: opaque(request_id).into(),
        idempotency_key: key.to_owned(),
        ..Default::default()
    }
}

pub(crate) fn opaque(id: Uuid) -> OpaqueId {
    OpaqueId {
        value: id.to_string(),
        ..Default::default()
    }
}

pub(crate) async fn gateway_id(
    pool: &PgPool,
    gateway: &GatewayGoldenFixture,
) -> Result<Uuid, AuthorityError> {
    Ok(sqlx::query_scalar(
        "SELECT binding.gateway_id FROM gateway_mailbox_bindings binding
         JOIN gateway_mailbox_binding_grants binding_grant ON binding_grant.binding_id = binding.id
         WHERE binding.mailbox_id = $1 AND binding_grant.id = $2",
    )
    .bind(gateway.mailbox_id.as_uuid())
    .bind(gateway.grant_id)
    .fetch_one(pool)
    .await?)
}
