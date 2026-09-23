//! Post-commit ingress response-loss probe for the cooking acceptance path.
//!
//! The proxy forwards one exact bounded request to real Caddy, waits for the
//! complete response, then closes the client socket before sending response
//! bytes. SQL is read-only evidence for durable publication and delivery.

use base64::Engine as _;
use runtime_types::RunId;
use rustls::{ClientConfig, RootCertStore, pki_types::CertificateDer};
use sqlx::PgPool;
use std::{
    fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};
use tokio_rustls::{TlsConnector, rustls::pki_types::ServerName};
use uuid::Uuid;

use super::cooking::CookingRun;

const MAX_HTTP_MESSAGE_BYTES: usize = 64 * 1024;

pub struct IngressLossContext<'a> {
    pub pool: &'a PgPool,
    pub mailbox_id: Uuid,
    pub caddy_public_url: &'a str,
    pub inbound_credential: &'a str,
    pub timeout: Duration,
}

/// One-shot proxy state. The listener forwards only the accepted request and
/// never writes the upstream response to its client.
pub struct IngressLossProxy {
    listener: TcpListener,
    upstream: SocketAddr,
    upstream_host: String,
    upstream_tls: bool,
    ca_path: Option<PathBuf>,
    url: String,
    committed: Option<oneshot::Sender<()>>,
}

impl IngressLossProxy {
    pub async fn bind(
        caddy_public_url: &str,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let ca_path = std::env::var_os("HEPHAESTUS_CADDY_TEST_CA_CERT").map(PathBuf::from);
        Self::bind_with_ca(caddy_public_url, ca_path).await
    }

    async fn bind_with_ca(
        caddy_public_url: &str,
        ca_path: Option<PathBuf>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let upstream_url = reqwest::Url::parse(caddy_public_url)?;
        let host = upstream_url.host_str().ok_or("Caddy URL has no host")?;
        let upstream_host = host.to_owned();
        let port = upstream_url
            .port_or_known_default()
            .ok_or("Caddy URL has no port")?;
        let upstream_tls = upstream_url.scheme() == "https";
        if !upstream_tls && upstream_url.scheme() != "http" {
            return Err(format!("unsupported Caddy URL scheme: {}", upstream_url.scheme()).into());
        }
        let upstream = tokio::net::lookup_host((host, port))
            .await?
            .next()
            .ok_or("Caddy host did not resolve")?;
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let url = format!("http://{}/gateway/cooking/telegram", listener.local_addr()?);
        Ok(Self {
            listener,
            upstream,
            upstream_host,
            upstream_tls,
            ca_path,
            url,
            committed: None,
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn arm_commit_signal(&mut self) -> oneshot::Receiver<()> {
        let (sender, receiver) = oneshot::channel();
        self.committed = Some(sender);
        receiver
    }

    pub async fn forward_and_drop(
        mut self,
        timeout: Duration,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let operation = async {
            let (mut client, _) = self.listener.accept().await?;
            let request = read_http_message(&mut client, timeout).await?;
            let upstream = TcpStream::connect(self.upstream).await?;
            if self.upstream_tls {
                let ca_path = self
                    .ca_path
                    .as_deref()
                    .ok_or("HEPHAESTUS_CADDY_TEST_CA_CERT is required for HTTPS ingress loss")?;
                let connector = caddy_tls_connector(ca_path)?;
                let server_name = ServerName::try_from(self.upstream_host.clone())
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
                let upstream = connector.connect(server_name, upstream).await?;
                self.forward_upstream(&mut client, upstream, request, timeout)
                    .await?;
            } else {
                self.forward_upstream(&mut client, upstream, request, timeout)
                    .await?;
            }
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        };
        tokio::time::timeout(timeout, operation).await??;
        Ok(())
    }

    async fn forward_upstream<S>(
        &mut self,
        client: &mut TcpStream,
        mut upstream: S,
        request: Vec<u8>,
        timeout: Duration,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        upstream.write_all(&request).await?;
        let response = read_http_message(&mut upstream, timeout).await?;
        let status = response
            .split(|byte| *byte == b' ')
            .nth(1)
            .and_then(|value| std::str::from_utf8(value).ok())
            .unwrap_or_default();
        assert_eq!(status, "200", "Caddy did not accept the forwarded ingress");
        if let Some(signal) = self.committed.take() {
            let _ = signal.send(());
        }
        // Deliberately close before writing even one response byte. The
        // caller therefore observes a transport error after the upstream
        // commit.
        client.shutdown().await?;
        Ok(())
    }
}

fn caddy_tls_connector(
    ca_path: &Path,
) -> Result<TlsConnector, Box<dyn std::error::Error + Send + Sync>> {
    let pem = fs::read_to_string(ca_path)?;
    let encoded = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<String>();
    let der = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from(der))?;
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Sends Alice update 42 through the loss proxy concurrently with Bob update
/// 43, then retries 42 directly against Caddy. The caller keeps its existing
/// explicit replay, which raises Alice's duplicate count from one to two.
pub async fn exercise(
    ctx: &IngressLossContext<'_>,
    proxy: IngressLossProxy,
) -> Result<(CookingRun, CookingRun), Box<dyn std::error::Error + Send + Sync>> {
    let baseline_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
            .bind(ctx.mailbox_id)
            .fetch_one(ctx.pool)
            .await?;
    let mut proxy = proxy;
    let committed = proxy.arm_commit_signal();
    let proxy_url = proxy.url().to_owned();
    let proxy_timeout = ctx.timeout;
    let proxy_task = tokio::spawn(async move {
        tokio::time::timeout(proxy_timeout, proxy.forward_and_drop(proxy_timeout)).await?
    });
    let client = super::cooking::caddy_gateway_client_with_timeout(ctx.timeout);
    let request = client
        .post(&proxy_url)
        .header("host", caddy_host(ctx.caddy_public_url)?)
        .header("x-telegram-bot-api-secret-token", ctx.inbound_credential)
        .json(&serde_json::json!({
            "update_id": 42,
            "message": {"from": {"id": 1001}, "text": "pasta"}
        }))
        .send();
    let bob_request = client
        .post(format!(
            "{}/gateway/cooking/telegram",
            ctx.caddy_public_url.trim_end_matches('/')
        ))
        .header("x-telegram-bot-api-secret-token", ctx.inbound_credential)
        .json(&serde_json::json!({
            "update_id": 43,
            "message": {"from": {"id": 1002}, "text": "soup"}
        }))
        .send();
    let (request_result, bob_result, committed_result) =
        tokio::join!(request, bob_request, committed);
    assert!(
        request_result.is_err(),
        "response-loss client unexpectedly received a response"
    );
    let bob = bob_result?;
    assert_eq!(bob.status(), reqwest::StatusCode::OK);
    bob.bytes().await?;
    committed_result.expect("proxy observed the complete Caddy response");
    proxy_task.await??;

    let event_id = wait_for_single_accepted(ctx.pool, ctx.mailbox_id, 42, ctx.timeout).await?;
    wait_for_single_accepted(ctx.pool, ctx.mailbox_id, 43, ctx.timeout).await?;
    assert_eq!(
        mailbox_event_count(ctx.pool, ctx.mailbox_id).await?,
        baseline_events + 2
    );
    let runs_before = wait_for_delivery_attempt(ctx.pool, event_id, ctx.timeout).await?;

    let retry = client
        .post(format!(
            "{}/gateway/cooking/telegram",
            ctx.caddy_public_url.trim_end_matches('/')
        ))
        .header("x-telegram-bot-api-secret-token", ctx.inbound_credential)
        .json(&serde_json::json!({
            "update_id": 42,
            "message": {"from": {"id": 1001}, "text": "pasta"}
        }))
        .send()
        .await?;
    assert_eq!(retry.status(), reqwest::StatusCode::OK);
    retry.bytes().await?;
    wait_for_duplicate(ctx.pool, ctx.mailbox_id, 42, 1, ctx.timeout).await?;
    assert_eq!(event_run_count(ctx.pool, event_id).await?, runs_before);
    assert_eq!(
        mailbox_event_count(ctx.pool, ctx.mailbox_id).await?,
        baseline_events + 2
    );
    let alice_run = wait_for_event_run(ctx.pool, ctx.mailbox_id, 42, ctx.timeout).await?;
    let bob_run = wait_for_event_run(ctx.pool, ctx.mailbox_id, 43, ctx.timeout).await?;
    Ok((alice_run, bob_run))
}

async fn wait_for_single_accepted(
    pool: &PgPool,
    mailbox_id: Uuid,
    update_id: u64,
    timeout: Duration,
) -> Result<Uuid, Box<dyn std::error::Error + Send + Sync>> {
    let key = format!("telegram-update-{update_id}");
    Ok(tokio::time::timeout(timeout, async {
        loop {
            let rows: Vec<(Uuid,)> = sqlx::query_as(
                "SELECT event_id FROM gateway_mailbox_publications
                  WHERE mailbox_id = $1 AND deduplication_key = $2
                    AND outcome = 'accepted'",
            )
            .bind(mailbox_id)
            .bind(&key)
            .fetch_all(pool)
            .await?;
            if rows.len() == 1 {
                return Ok::<Uuid, sqlx::Error>(rows[0].0);
            }
            assert!(
                rows.is_empty(),
                "response-loss ingress created multiple accepted publications"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??)
}

async fn wait_for_duplicate(
    pool: &PgPool,
    mailbox_id: Uuid,
    update_id: u64,
    expected: i64,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let key = format!("telegram-update-{update_id}");
    tokio::time::timeout(timeout, async {
        loop {
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM gateway_mailbox_publications
                  WHERE mailbox_id = $1 AND deduplication_key = $2
                    AND outcome = 'duplicate'",
            )
            .bind(mailbox_id)
            .bind(&key)
            .fetch_one(pool)
            .await?;
            if count == expected {
                return Ok::<(), sqlx::Error>(());
            }
            assert!(count <= expected);
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??;
    Ok(())
}

async fn mailbox_event_count(pool: &PgPool, mailbox_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
        .bind(mailbox_id)
        .fetch_one(pool)
        .await
}

async fn event_run_count(pool: &PgPool, event_id: Uuid) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM mailbox_delivery_attempts WHERE event_id = $1")
        .bind(event_id)
        .fetch_one(pool)
        .await
}

async fn wait_for_delivery_attempt(
    pool: &PgPool,
    event_id: Uuid,
    timeout: Duration,
) -> Result<i64, Box<dyn std::error::Error + Send + Sync>> {
    Ok(tokio::time::timeout(timeout, async {
        loop {
            let count = event_run_count(pool, event_id).await?;
            if count != 0 {
                return Ok::<i64, sqlx::Error>(count);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??)
}

async fn wait_for_event_run(
    pool: &PgPool,
    mailbox_id: Uuid,
    update_id: u64,
    timeout: Duration,
) -> Result<CookingRun, Box<dyn std::error::Error + Send + Sync>> {
    let key = format!("telegram-update-{update_id}");
    Ok(tokio::time::timeout(timeout, async {
        loop {
            let row: Option<(Uuid, Uuid)> = sqlx::query_as(
                "SELECT publication.event_id, attempt.run_id
                   FROM gateway_mailbox_publications AS publication
                   JOIN mailbox_delivery_attempts AS attempt
                     ON attempt.event_id = publication.event_id
                   JOIN mailbox_deliveries AS delivery
                     ON delivery.event_id = publication.event_id
                   JOIN runs AS run
                     ON run.id = attempt.run_id
                  WHERE publication.mailbox_id = $1
                    AND publication.deduplication_key = $2
                    AND publication.outcome = 'accepted'
                    AND run.state = 'cleaned_up'
                    AND run.outcome = 'succeeded'
                    AND delivery.disposition = 'delivered'
                    AND NOT EXISTS (
                        SELECT 1
                          FROM mailbox_delivery_attempts AS newer
                         WHERE newer.event_id = attempt.event_id
                           AND (newer.attempt_number, newer.id)
                               > (attempt.attempt_number, attempt.id)
                    )
                  ORDER BY attempt.attempt_number DESC, attempt.id DESC
                  LIMIT 1",
            )
            .bind(mailbox_id)
            .bind(&key)
            .fetch_optional(pool)
            .await?;
            if let Some((event_id, run_id)) = row {
                return Ok::<CookingRun, sqlx::Error>(CookingRun {
                    event_id,
                    run_id: RunId::from_uuid(run_id),
                });
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??)
}

fn caddy_host(url: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let parsed = reqwest::Url::parse(url)?;
    let host = parsed.host_str().ok_or("Caddy URL has no host")?;
    Ok(parsed
        .port()
        .map_or_else(|| host.to_owned(), |port| format!("{host}:{port}")))
}

async fn read_http_message<S>(
    stream: &mut S,
    timeout: Duration,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + Unpin,
{
    let mut data = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end;
    loop {
        let read = tokio::time::timeout(timeout, stream.read(&mut chunk)).await??;
        if read == 0 {
            return Err(
                io::Error::new(io::ErrorKind::UnexpectedEof, "HTTP headers truncated").into(),
            );
        }
        data.extend_from_slice(&chunk[..read]);
        if data.len() > MAX_HTTP_MESSAGE_BYTES {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "HTTP message exceeds bound").into(),
            );
        }
        if let Some(position) = data.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = position + 4;
            break;
        }
    }
    let headers = &data[..header_end];
    let content_length = headers
        .split(|byte| *byte == b'\n')
        .find_map(|line| {
            let line = std::str::from_utf8(line).ok()?.trim();
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or("HTTP message lacks bounded Content-Length")?;
    let message_end = header_end
        .checked_add(content_length)
        .filter(|end| *end <= MAX_HTTP_MESSAGE_BYTES)
        .ok_or("HTTP message exceeds bound")?;
    while data.len() < message_end {
        let remaining = message_end - data.len();
        let read_len = remaining.min(chunk.len());
        let read = tokio::time::timeout(timeout, stream.read(&mut chunk[..read_len])).await??;
        if read == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "HTTP body truncated").into());
        }
        data.extend_from_slice(&chunk[..read]);
    }
    data.truncate(message_end);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
    use tokio_rustls::TlsAcceptor;

    async fn start_tls_server() -> (String, String, tokio::task::JoinHandle<()>) {
        let mut ca_parameters = CertificateParams::default();
        ca_parameters.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let ca_key = KeyPair::generate().expect("generate ingress-loss CA key");
        let ca = ca_parameters
            .self_signed(&ca_key)
            .expect("self-sign ingress-loss CA");
        let leaf_parameters =
            CertificateParams::new(vec![String::from("127.0.0.1")]).expect("loopback identity");
        let leaf_key = KeyPair::generate().expect("generate ingress-loss leaf key");
        let leaf = leaf_parameters
            .signed_by(&leaf_key, &ca, &ca_key)
            .expect("sign ingress-loss leaf");
        let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ingress-loss safe TLS protocol versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(leaf.der().to_vec())],
            rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .expect("ingress-loss TLS server configuration");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ingress-loss TLS server");
        let port = listener
            .local_addr()
            .expect("ingress-loss TLS server address")
            .port();
        let server = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let Ok(mut stream) = TlsAcceptor::from(Arc::new(tls)).accept(stream).await else {
                return;
            };
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            loop {
                let Ok(read) = stream.read(&mut chunk).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                request.extend_from_slice(&chunk[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        });
        (format!("https://127.0.0.1:{port}"), ca.pem(), server)
    }

    #[tokio::test]
    async fn https_proxy_requires_joined_ca_before_commit() {
        let temp = tempfile::tempdir().expect("ingress-loss TLS fixture directory");
        let (trusted_url, trusted_ca, trusted_server) = start_tls_server().await;
        let trusted_path = temp.path().join("trusted-ca.pem");
        fs::write(&trusted_path, trusted_ca).expect("write trusted ingress-loss CA");
        let mut proxy = IngressLossProxy::bind_with_ca(&trusted_url, Some(trusted_path.clone()))
            .await
            .expect("bind trusted HTTPS ingress-loss proxy");
        let committed = proxy.arm_commit_signal();
        let proxy_url = proxy.url().to_owned();
        let proxy_task =
            tokio::spawn(async move { proxy.forward_and_drop(Duration::from_secs(5)).await });
        let response = reqwest::Client::new()
            .post(proxy_url)
            .body("trusted ingress")
            .send()
            .await;
        assert!(
            response.is_err(),
            "response-loss client received a response"
        );
        committed.await.expect("trusted HTTPS proxy commit signal");
        proxy_task
            .await
            .expect("trusted HTTPS proxy task")
            .expect("trusted HTTPS proxy forwarding");
        trusted_server.await.expect("trusted HTTPS server task");

        let (wrong_url, _wrong_ca, wrong_server) = start_tls_server().await;
        let mut wrong_proxy = IngressLossProxy::bind_with_ca(&wrong_url, Some(trusted_path))
            .await
            .expect("bind wrong-CA HTTPS ingress-loss proxy");
        let wrong_committed = wrong_proxy.arm_commit_signal();
        let wrong_proxy_url = wrong_proxy.url().to_owned();
        let wrong_proxy_task =
            tokio::spawn(async move { wrong_proxy.forward_and_drop(Duration::from_secs(5)).await });
        let wrong_response = reqwest::Client::new()
            .post(wrong_proxy_url)
            .body("wrong CA ingress")
            .send()
            .await;
        assert!(
            wrong_response.is_err(),
            "wrong-CA client received a response"
        );
        assert!(
            !matches!(
                tokio::time::timeout(Duration::from_secs(1), wrong_committed).await,
                Ok(Ok(()))
            ),
            "wrong CA must fail before the commit signal"
        );
        assert!(
            wrong_proxy_task
                .await
                .expect("wrong-CA proxy task")
                .is_err(),
            "wrong CA must fail the HTTPS upstream handshake"
        );
        wrong_server.await.expect("wrong-CA HTTPS server task");
    }
}
