use super::*;

/// Controlled model response barrier used by the cooking update scenario.
/// The model request enters before the v1 drain and resumes only after the
/// test has observed the durable drain state.
pub struct CookingUpdateBarrier {
    pub entered: AtomicBool,
    pub entered_notify: Notify,
    pub release: Notify,
}

impl CookingUpdateBarrier {
    pub fn new() -> Self {
        Self {
            entered: AtomicBool::new(false),
            entered_notify: Notify::new(),
            release: Notify::new(),
        }
    }

    pub async fn wait_entered(&self) {
        while !self.entered.load(Ordering::Acquire) {
            self.entered_notify.notified().await;
        }
    }

    pub fn mark_entered(&self) {
        self.entered.store(true, Ordering::Release);
        self.entered_notify.notify_one();
    }

    pub fn release(&self) {
        self.release.notify_one();
    }
}

#[cfg(test)]
#[tokio::test]
pub async fn cooking_update_barrier_waits_for_release() {
    let barrier = Arc::new(CookingUpdateBarrier::new());
    let upstream = Arc::new(BrokeredTlsUpstream {
        adapter: Arc::new(DenyingBrokerAdapter),
        rule_adapters: Arc::new(std::collections::HashMap::new()),
        cooking_registry: None,
        observed: Arc::new(AtomicBool::new(false)),
        server: tokio::spawn(async {}),
        update_barrier: Some(Arc::clone(&barrier)),
        revocation_barrier: None,
    });
    let waiter = Arc::clone(&upstream);
    let task = tokio::spawn(async move {
        waiter.wait_update_v1_entered().await;
    });
    barrier.mark_entered();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("barrier waiter timeout")
        .expect("barrier waiter task");
    upstream.release_update_v1();
}

#[cfg(test)]
#[tokio::test]
pub async fn cooking_update_barrier_buffers_release_before_waiter_registration() {
    let barrier = CookingUpdateBarrier::new();
    barrier.mark_entered();
    barrier.wait_entered().await;
    barrier.release();
    tokio::time::timeout(Duration::from_secs(1), barrier.release.notified())
        .await
        .expect("release notification should retain its permit");
}

impl BrokeredTlsUpstream {
    pub async fn wait_update_v1_entered(&self) {
        if let Some(barrier) = &self.update_barrier {
            barrier.wait_entered().await;
        } else {
            panic!("cooking update barrier is not enabled");
        }
    }

    pub async fn wait_revocation_entered(&self) {
        if let Some(barrier) = &self.revocation_barrier {
            barrier.wait_entered().await;
        } else {
            panic!("cooking revocation barrier is not enabled");
        }
    }

    pub fn release_revocation(&self) {
        if let Some(barrier) = &self.revocation_barrier {
            barrier.release();
        } else {
            panic!("cooking revocation barrier is not enabled");
        }
    }

    pub fn release_update_v1(&self) {
        if let Some(barrier) = &self.update_barrier {
            barrier.release();
        } else {
            panic!("cooking update barrier is not enabled");
        }
    }

    pub async fn start(rule: BrokeredSecretRule) -> Self {
        let _already_installed = rustls::crypto::ring::default_provider().install_default();
        let mut ca_parameters = rcgen::CertificateParams::default();
        ca_parameters.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = rcgen::KeyPair::generate().expect("generate golden upstream CA key");
        let ca = ca_parameters
            .self_signed(&ca_key)
            .expect("self-sign golden upstream CA");
        let leaf_parameters = rcgen::CertificateParams::new(vec![String::from("api.example.test")])
            .expect("golden upstream DNS identity");
        let leaf_key = rcgen::KeyPair::generate().expect("generate golden upstream leaf key");
        let leaf = leaf_parameters
            .signed_by(&leaf_key, &ca, &ca_key)
            .expect("sign golden upstream leaf");
        let tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
            )
            .expect("golden upstream TLS configuration");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind golden TLS upstream");
        let port = listener
            .local_addr()
            .expect("golden TLS upstream address")
            .port();
        let observed = Arc::new(AtomicBool::new(false));
        let observed_server = Arc::clone(&observed);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept golden TLS request");
            let mut stream = TlsAcceptor::from(Arc::new(tls))
                .accept(stream)
                .await
                .expect("verify golden TLS handshake");
            let mut request = Vec::with_capacity(512);
            loop {
                let mut chunk = [0_u8; 512];
                let read = stream.read(&mut chunk).await.expect("read golden request");
                assert_ne!(read, 0, "golden TLS request ended before headers");
                request.extend_from_slice(&chunk[..read]);
                assert!(
                    request.len() <= 8 * 1024,
                    "golden TLS request exceeded bound"
                );
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = std::str::from_utf8(&request).expect("golden HTTPS request is UTF-8");
            assert!(request.starts_with("GET /v1/probe HTTP/1.1\r\n"));
            assert!(request.contains(&format!(
                "authorization: Bearer {BROKERED_E2E_SENTINEL}\r\n"
            )));
            assert!(!request.contains("heph-placeholder:"));
            observed_server.store(true, Ordering::SeqCst);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\nbrokered-e2e-ok")
                .await
                .expect("respond to golden brokered request");
        });
        let adapter = BrokeredHttpsAdapterRegistry::test_only_local_trusted(
            rule,
            port,
            "127.0.0.1".parse().expect("golden loopback pin"),
            ca.pem().as_bytes(),
        )
        .expect("configure locally trusted pinned broker registry");
        let adapter: Arc<dyn secret_application::BrokerAdapter> = Arc::new(adapter);
        Self {
            adapter: Arc::clone(&adapter),
            cooking_registry: None,
            rule_adapters: Arc::new(std::collections::HashMap::from([(
                BROKERED_E2E_RULE_ID,
                adapter,
            )])),
            observed,
            server,
            update_barrier: None,
            revocation_barrier: None,
        }
    }

    pub fn adapter(&self) -> Arc<dyn secret_application::BrokerAdapter> {
        Arc::clone(&self.adapter)
    }

    pub fn register_rule_copies(&self, copies: &[(uuid::Uuid, uuid::Uuid)]) {
        self.cooking_registry
            .as_ref()
            .expect("cooking TLS registry")
            .register_rule_copies(copies);
    }

    /// Combines rule-keyed adapters before a restart so canonical and crash
    /// instances share one broker boundary without destination guessing.
    pub fn combined_adapter(&self, other: &Self) -> Arc<dyn secret_application::BrokerAdapter> {
        let mut adapters = self.cooking_registry.as_ref().map_or_else(
            || (*self.rule_adapters).clone(),
            |registry| registry.snapshot(),
        );
        adapters.extend(
            other
                .cooking_registry
                .as_ref()
                .map_or_else(
                    || (*other.rule_adapters).clone(),
                    |registry| registry.snapshot(),
                )
                .iter()
                .map(|(id, adapter)| (*id, Arc::clone(adapter))),
        );
        Arc::new(cooking::CookingAdapters::new(adapters))
    }

    pub async fn wait_complete(self) -> Result<(), tokio::task::JoinError> {
        self.server.await
    }

    pub async fn assert_substituted_request(self) {
        tokio::time::timeout(Duration::from_secs(10), self.server)
            .await
            .expect("golden TLS upstream request timeout")
            .expect("golden TLS upstream task");
        assert!(
            self.observed.load(Ordering::SeqCst),
            "upstream did not receive the substituted HTTPS request"
        );
    }
}
