use super::IngressLossProxy;
use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
use rustls::pki_types::CertificateDer;
use std::{fs, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
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
