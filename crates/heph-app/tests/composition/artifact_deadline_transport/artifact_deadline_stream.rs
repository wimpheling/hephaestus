use super::{opaque, session_token};
use connectrpc::{
    client::{CallOptions, SharedHttp2Connection},
    error::ConnectError,
};
use identity_domain::BrowserSessionSid;
use rpc_proto::{
    connect::hephaestus::artifact::v1::ArtifactServiceClient,
    messages::hephaestus::artifact::v1::StreamArtifactRequest,
};
use uuid::Uuid;

pub(super) async fn artifact_stream_error(
    client: &ArtifactServiceClient<SharedHttp2Connection>,
    artifact: Uuid,
    user: Uuid,
    sid: BrowserSessionSid,
    timeout_ms: &str,
) -> ConnectError {
    let options = CallOptions::default()
        .with_header(
            "authorization",
            format!("Bearer {}", session_token(user, sid)),
        )
        .with_header("connect-timeout-ms", timeout_ms);
    match client
        .stream_artifact_with_options(
            StreamArtifactRequest {
                artifact_id: opaque(artifact).into(),
                max_total_bytes: 1024,
                max_chunk_bytes: 1024,
                ..Default::default()
            },
            options,
        )
        .await
    {
        Ok(mut stream) => stream
            .message()
            .await
            .expect_err("deadline must fail the artifact response stream"),
        Err(error) => error,
    }
}
