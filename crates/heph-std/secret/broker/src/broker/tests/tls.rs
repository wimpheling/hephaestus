// Secret broker test scenarios.
use super::support::*;

#[tokio::test]
async fn pinned_transport_accepts_a_ca_signed_local_tls_upstream() {
    let _provider = rustls::crypto::ring::default_provider().install_default();
    let mut ca_parameters = rcgen::CertificateParams::default();
    ca_parameters.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().expect("generate fixture CA key");
    let ca = ca_parameters.self_signed(&ca_key).expect("sign fixture CA");
    let leaf_parameters = rcgen::CertificateParams::new(vec![String::from("api.example.test")])
        .expect("fixture DNS name");
    let leaf_key = rcgen::KeyPair::generate().expect("generate fixture leaf key");
    let leaf = leaf_parameters
        .signed_by(&leaf_key, &ca, &ca_key)
        .expect("sign fixture leaf");
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
            rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .expect("fixture TLS server");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture");
    let port = listener.local_addr().expect("fixture address").port();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept fixture");
        let mut stream = tokio_rustls::TlsAcceptor::from(Arc::new(tls))
            .accept(stream)
            .await
            .expect("trusted handshake");
        let mut request = [0_u8; 1024];
        let read = stream.read(&mut request).await.expect("read request");
        assert!(
            std::str::from_utf8(&request[..read])
                .expect("HTTP")
                .contains("authorization: Bearer trusted-sentinel")
        );
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await
            .expect("respond");
    });
    let transport = ReqwestPinnedHttpsTransport::test_only_local_trusted_pin(
        "api.example.test",
        port,
        "127.0.0.1".parse().expect("loopback"),
        ca.pem().as_bytes(),
    )
    .expect("trusted pin");
    let result = transport
        .send(UpstreamHttpsRequest {
            method: BrokeredHttpsMethod::Get,
            destination: format!("api.example.test:{port}"),
            path_and_query: String::from("/"),
            headers: vec![BrokeredHttpsHeader {
                name: String::from("authorization"),
                value: String::from("Bearer trusted-sentinel"),
            }],
            body: Vec::new(),
        })
        .await;
    server.await.expect("fixture task");
    assert_eq!(result.expect("trusted request").body, b"ok");
}
#[tokio::test]
#[ignore = "the daemon golden owns the full locally trusted TLS composition"]
// PEM fixtures make the trusted-handshake exchange auditable in one test.
#[allow(
    clippy::items_after_statements,
    clippy::needless_raw_string_hashes,
    clippy::too_many_lines
)]
async fn pinned_transport_uses_a_locally_trusted_certificate_before_substitution() {
    let _already_installed = rustls::crypto::ring::default_provider().install_default();
    const CERTIFICATE: &str = r#"-----BEGIN CERTIFICATE-----
MIIDNDCCAhygAwIBAgIUZQFCF7Lpa/1CzKr57uddVI6nC5QwDQYJKoZIhvcNAQEL
BQAwGzEZMBcGA1UEAwwQYXBpLmV4YW1wbGUudGVzdDAeFw0yNjA4MTAxMDUyMzha
Fw0yNjA4MTIxMDUyMzhaMBsxGTAXBgNVBAMMEGFwaS5leGFtcGxlLnRlc3QwggEi
MA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQDhB3mX5m5CW5wqbSNg3oUDiqbD
1Dyd13dqGbzhBlpcGXL9LfOUdh9jmySCFIVPVJON1/9rGKowbsrzmihlTt2ER9i4
iXbMGwd1Ag/Vm5Ai3oOuCDxjfJ2DDG2PhvMR30HJ+eiByL2SGkkocqK08ufWz9mL
b61pBcsekR0YOjQPxqKBM0ZWIKJDJ+fvz/lPQRrI6C/Z+I8l3Mi8smmLUfY6W9wW
kS1kMAzlYVtUpIPLSUCQoN72XL1T8IFruMlwWepAsMT2F5XvqymJhnsEu6Kz3Ipo
BXPkvPqiFSzGoL0K5ZsSnilA8BDlFEAekVC/evw6iD4CCDKFOcjb+dKqpfQRAgMB
AAGjcDBuMB0GA1UdDgQWBBSQj6CnLfxWDjZ0Ecz012/kCcR1qDAfBgNVHSMEGDAW
gBSQj6CnLfxWDjZ0Ecz012/kCcR1qDAPBgNVHRMBAf8EBTADAQH/MBsGA1UdEQQU
MBKCEGFwaS5leGFtcGxlLnRlc3QwDQYJKoZIhvcNAQELBQADggEBAErq48Kwt8QD
9yKycUnfwwHVML5nZ4JMJHkF6q5qix4s4HuQqjly9zTE6Z+j+A7l1DqaxtKDxq/C
S56fCje43qi6ul/roZ1cu9M9whMOjZTd+esI0VNoc8prY3fsEynhB2gUFqukPibV
cZGIsonAltuPbyETK9Rk/rvqzWzj+62qW3KdpQb1KKMzpkJWK9aRDLEPTYKRLotn
tdHZMepiiWXX/hxcNskvKZ04vkSMis7W2WnMfjMXj3fu8mLfizD71MEpCujkUf+V
U1bH9+FgY/nCdrGg5JfrYANRkxrddqJrRam/Y1lksLceJ217p2l3HIHFgvWNmyuj
UvdjlrooO0Q=
-----END CERTIFICATE-----
"#;
    const PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQDhB3mX5m5CW5wq
bSNg3oUDiqbD1Dyd13dqGbzhBlpcGXL9LfOUdh9jmySCFIVPVJON1/9rGKowbsrz
mihlTt2ER9i4iXbMGwd1Ag/Vm5Ai3oOuCDxjfJ2DDG2PhvMR30HJ+eiByL2SGkko
cqK08ufWz9mLb61pBcsekR0YOjQPxqKBM0ZWIKJDJ+fvz/lPQRrI6C/Z+I8l3Mi8
smmLUfY6W9wWkS1kMAzlYVtUpIPLSUCQoN72XL1T8IFruMlwWepAsMT2F5XvqymJ
hnsEu6Kz3IpoBXPkvPqiFSzGoL0K5ZsSnilA8BDlFEAekVC/evw6iD4CCDKFOcjb
+dKqpfQRAgMBAAECgf8BUYnk2fTLbr37gagzaRpeavGPNC8mvZx1grEYoHDnGT8T
PLGwrQCCFOah3HzrehNjQWC9v+c/YWbPBpg1/8BMhh8+9Y88ouvoQ5rUJZUynxsm
aeXwr0o8+lWqAaBq+mPoSw6RtBtoP78t/X32kXXKTG15462gb+hAxZjmC3FCpmR8
YI8zuJ2oFIEn7IrIMm5nFwKFYy53kW9FM02QJelzUP7UxL3qcQuQuhD7vtw8DtG1
BHqN7JQSdxU+jfH4ASsL3LFkYJ92EkbGFSVX/xQbhTCUvHOdXw0trHBw5UNEJk/m
D2c51IMwukvI/4C89Id1oX7e9cH9Vwx9lcZrnM8CgYEA/JnrNCQsAR/dNTdmuZQP
XcYn4ckPCc6XNZBXHTKnOf/HecYq4Ijcr9xnatAn03vOB9L2RdWc46/nGuDN89Xq
omG4DeLek0TGgVo0N2Q2wdni50S1akJ0WDopdfdfzCL+r28ZSYU30RRSa5+kk0hm
l4TdmmkfKGTytT9BM44Nk/MCgYEA5A6Vr1dpC1cvNRnyHJJiq/8kkHfG84IBBBVO
yguvAL5ISSNZ2BFaBoU4zKSXXl1yC4Q1jNv+hPHu7XN1JvhX6dnP4Qc2c4m8F/IK
pYg8zx5xkdqfzcCBmews+JYeKqGEOLlRWZPk5v0ZfPdmP9ORc0UHo0soaA+V2zCL
SNpxTOsCgYAGN4+XZ/CBUpRyM9veY2uBZlgi8XziQ+hq1BOgz1dYURhKwfraLeQo
m+cbtOXWCa0HekS/cUN8Qx8QBUpsXu54cqlCBjxuKXotQtgYKOpEGSXBhWplpB8S
8NOGaME91/qmvLhFm/bEuZhRt8soKNcFkaqWm1G9/8YNefIT38IrywKBgQDPM9Eb
9jcibpwdiu1GuFmeG7qE586C/+Mcap+jQupFqpzSlqPShDFfKztn80L0IcK0Y/Kj
gF3HPGjwpK4UMh7uAU+2DG+Umdejie3SZ+2X4Pkeo5v9vKIDz2ksknfmE8mmH/mh
gZW/qMW3nK/x3a+RI27FFkwn/8EP3RMvpgi68QKBgQCe7cxdH39iXs8cwmIF7R8I
8FffEFT0s/QgLcUdhalZkNJQPwMrnn9tSigmfMLvRexCgdtacxHVCZLvnr05GSmt
tDQ4rbs8xs4S+H1ZU5kliFr+O0WjoDWLsxpraO+1pe3vDtMuaA287E56t3Lk/8jk
EzTjqb4SjCJxuxqQPKdVVg==
-----END PRIVATE KEY-----
"#;
    let certificates = rustls_pemfile::certs(&mut CERTIFICATE.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .expect("fixture certificate");
    let key = rustls_pemfile::private_key(&mut PRIVATE_KEY.as_bytes())
        .expect("fixture private key")
        .expect("one fixture private key");
    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certificates, key)
        .expect("fixture TLS configuration");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind trusted TLS fixture");
    let port = listener.local_addr().expect("fixture address").port();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept TLS client");
        let mut stream = tokio_rustls::TlsAcceptor::from(Arc::new(server_config))
            .accept(stream)
            .await
            .expect("complete trusted TLS handshake");
        let mut request = vec![0_u8; 4096];
        let read = stream.read(&mut request).await.expect("read HTTPS request");
        assert!(
            std::str::from_utf8(&request[..read])
                .expect("HTTP request UTF-8")
                .contains("authorization: Bearer locally-trusted-sentinel\\r\\n")
        );
        stream
            .write_all(
                b"HTTP/1.1 200 OK\\r\\nContent-Length: 2\\r\\nConnection: close\\r\\n\\r\\nok",
            )
            .await
            .expect("write HTTPS response");
    });
    let transport = ReqwestPinnedHttpsTransport::test_only_local_trusted_pin(
        "api.example.test",
        port,
        "127.0.0.1".parse().expect("loopback address"),
        CERTIFICATE.as_bytes(),
    )
    .expect("trusted local transport");
    let response = transport
        .send(UpstreamHttpsRequest {
            method: BrokeredHttpsMethod::Get,
            destination: format!("api.example.test:{port}"),
            path_and_query: String::from("/trusted"),
            headers: vec![BrokeredHttpsHeader {
                name: String::from("authorization"),
                value: String::from("Bearer locally-trusted-sentinel"),
            }],
            body: Vec::new(),
        })
        .await;
    let server_result = server.await;
    server_result.expect("trusted TLS fixture task");
    let response = response.expect("trusted HTTPS response");
    assert_eq!(response.status, BrokerStatus::Succeeded);
    assert_eq!(response.body, b"ok");
}
