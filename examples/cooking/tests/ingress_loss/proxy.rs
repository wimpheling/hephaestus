use super::read_http_message;
use base64::Engine as _;
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
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};
use tokio_rustls::{TlsConnector, rustls::pki_types::ServerName};
use uuid::Uuid;

pub(crate) const MAX_HTTP_MESSAGE_BYTES: usize = 64 * 1024;

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

    pub(crate) async fn bind_with_ca(
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

pub(crate) fn caddy_tls_connector(
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
