use super::{BuildError, CookingBuildContext};
use identity_domain::{BrowserSessionSid, UserId};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rpc_proto::{
    connect::hephaestus::{artifact::v1::ArtifactServiceClient, build::v1::BuildServiceClient},
    messages::hephaestus::common::v1::OpaqueId,
};
use std::time::Duration;
use uuid::Uuid;

pub(crate) fn rpc_build_client(
    context: &CookingBuildContext<'_>,
    audience: &str,
) -> Result<BuildServiceClient<connectrpc::client::HttpClient>, BuildError> {
    client_with_audience(context, audience)
        .map(|(transport, config)| BuildServiceClient::new(transport, config))
}

pub(crate) fn rpc_artifact_client(
    context: &CookingBuildContext<'_>,
) -> Result<ArtifactServiceClient<connectrpc::client::HttpClient>, BuildError> {
    client_with_audience(
        context,
        "/hephaestus.artifact.v1.ArtifactService/GetArtifactPreview",
    )
    .map(|(transport, config)| ArtifactServiceClient::new(transport, config))
}

pub(crate) fn rpc_release_client(
    context: &CookingBuildContext<'_>,
    audience: &str,
) -> Result<
    rpc_proto::connect::hephaestus::release::v1::ReleaseServiceClient<
        connectrpc::client::HttpClient,
    >,
    BuildError,
> {
    client_with_audience(context, audience).map(|(transport, config)| {
        rpc_proto::connect::hephaestus::release::v1::ReleaseServiceClient::new(transport, config)
    })
}

pub(crate) fn client_with_audience(
    context: &CookingBuildContext<'_>,
    audience: &str,
) -> Result<
    (
        connectrpc::client::HttpClient,
        connectrpc::client::ClientConfig,
    ),
    BuildError,
> {
    let uri = format!("http://{}", context.running.http_addr()).parse()?;
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!(
                "Bearer {}",
                (context.identity.rpc_token)(audience)
            ))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok((connectrpc::client::HttpClient::plaintext(), config))
}

pub(crate) fn request_context(
    operation: &str,
) -> rpc_proto::messages::hephaestus::common::v1::RequestContext {
    rpc_proto::messages::hephaestus::common::v1::RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: format!("cooking-blog-{operation}-{}", Uuid::new_v4()),
        ..Default::default()
    }
}

pub(crate) fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

pub(crate) fn response_id(value: Option<OpaqueId>, operation: &str) -> Result<Uuid, BuildError> {
    value
        .ok_or_else(|| invalid_state(&format!("{operation} returned no ID")))?
        .value
        .parse()
        .map_err(Into::into)
}

pub(crate) fn opaque_value(value: Option<&OpaqueId>) -> String {
    value.map_or_else(String::new, |id| id.value.clone())
}

pub(crate) fn invalid_state(message: &str) -> BuildError {
    message.to_owned().into()
}

pub(crate) fn outsider_assertion(
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
            "exp": now + 30,
            "jti": Uuid::new_v4().to_string(),
            "sid": outsider_browser_session.to_protocol_string()
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        )),
    )
    .expect("sign outsider artifact assertion")
}
