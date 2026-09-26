use super::{opaque, session_token};
use buffa::Message as _;
use bytes::Bytes;
use connectrpc::{client::CallOptions, error::ConnectError};
use identity_domain::BrowserSessionSid;
use rpc_proto::{
    connect::hephaestus::artifact::v1::ArtifactServiceClient,
    messages::hephaestus::{artifact::v1::StreamArtifactRequest, common::v1::RequestContext},
};
use sqlx::PgPool;
use std::{
    error::Error,
    fmt::{Display, Formatter},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const ARTIFACT_PATH: &str = "/hephaestus.artifact.v1.ArtifactService/StreamArtifact";

#[derive(Debug)]
pub(super) struct TransportError(String);

impl Display for TransportError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TransportError {}

impl TransportError {
    pub(super) fn from_display<E: Display>(error: E) -> Self {
        Self(error.to_string())
    }
}

pub(super) struct RawArtifactRequest {
    response: Option<h2::client::ResponseFuture>,
    receive_stream: Option<h2::RecvStream>,
    send_stream: h2::SendStream<Bytes>,
    connection: tokio::task::JoinHandle<Result<(), h2::Error>>,
}

impl RawArtifactRequest {
    pub(super) async fn await_headers(&mut self) {
        let response = self
            .response
            .take()
            .expect("raw artifact response is awaited once")
            .await
            .expect("raw artifact response headers");
        self.receive_stream = Some(response.into_body());
    }

    pub(super) fn reset(&mut self) {
        self.send_stream.send_reset(h2::Reason::CANCEL);
    }

    pub(super) async fn finish(self) {
        self.connection.abort();
        let _ = self.connection.await;
        drop(self.response);
        drop(self.receive_stream);
    }
}

pub(super) async fn start_raw_artifact_request(
    uri: axum::http::Uri,
    artifact: Uuid,
    user: Uuid,
    sid: BrowserSessionSid,
) -> RawArtifactRequest {
    let authority = uri
        .authority()
        .expect("artifact RPC URI has an authority")
        .as_str();
    let stream = tokio::net::TcpStream::connect(authority)
        .await
        .expect("connect raw artifact RPC");
    let (mut sender, connection) = h2::client::handshake(stream)
        .await
        .expect("handshake raw artifact RPC");
    let connection = tokio::spawn(connection);
    let request = http::Request::builder()
        .method("POST")
        .uri(format!("http://{authority}{ARTIFACT_PATH}"))
        .version(http::Version::HTTP_2)
        .header("content-type", "application/connect+proto")
        .header("connect-protocol-version", "1")
        .header("connect-timeout-ms", "5000")
        .header(
            "authorization",
            format!("Bearer {}", session_token(user, sid)),
        )
        .body(())
        .expect("build raw artifact RPC request");
    let (response, mut send_stream) = sender
        .send_request(request, false)
        .expect("send raw artifact RPC headers");
    let payload = rpc_proto::messages::hephaestus::artifact::v1::StreamArtifactRequest {
        artifact_id: opaque(artifact).into(),
        max_total_bytes: 1024,
        max_chunk_bytes: 1024,
        ..Default::default()
    }
    .encode_to_vec();
    let mut framed = Vec::with_capacity(5 + payload.len());
    framed.push(0);
    framed.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("artifact request fits Connect envelope")
            .to_be_bytes(),
    );
    framed.extend_from_slice(&payload);
    send_stream
        .send_data(Bytes::from(framed), true)
        .expect("send raw artifact RPC body");
    RawArtifactRequest {
        response: Some(response),
        receive_stream: None,
        send_stream,
        connection,
    }
}

pub(super) fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after Unix epoch")
        .as_secs()
        .try_into()
        .expect("test clock fits i64")
}

pub(super) fn request_context(key: &str) -> RequestContext {
    RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: key.to_owned(),
        ..Default::default()
    }
}

pub(super) async fn artifact_stream_error<T>(
    client: ArtifactServiceClient<T>,
    artifact: Uuid,
    user: Uuid,
    sid: BrowserSessionSid,
    timeout_ms: &str,
) -> ConnectError
where
    T: connectrpc::client::ClientTransport,
    T::ResponseBody: Send + Unpin,
    <T::ResponseBody as connectrpc::http_body::Body>::Error: std::fmt::Display,
{
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

pub(super) async fn wait_for_artifact_lock(pool: &PgPool) {
    tokio::time::timeout(Duration::from_millis(400), async {
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS (
                     SELECT 1 FROM pg_stat_activity
                     WHERE pid <> pg_backend_pid()
                       AND datname = current_database()
                       AND state = 'active'
                       AND wait_event_type = 'Lock'
                       AND query LIKE '%FROM release_artifacts%'
                 )",
            )
            .fetch_one(pool)
            .await
            .expect("inspect artifact authorization wait");
            if waiting {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("artifact authorization must reach a database lock wait");
}

pub(super) async fn wait_for_artifact_query_to_terminate(pool: &PgPool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let terminated: bool = sqlx::query_scalar(
                "SELECT NOT EXISTS (
                     SELECT 1 FROM pg_stat_activity
                     WHERE pid <> pg_backend_pid()
                       AND datname = current_database()
                       AND state = 'active'
                       AND query LIKE '%FROM release_artifacts%'
                 )",
            )
            .fetch_one(pool)
            .await
            .expect("inspect canceled artifact authorization query");
            if terminated {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("artifact authorization query must terminate after client drop");
}
