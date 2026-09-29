use crate::*;
use async_trait::async_trait;
use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use uuid::Uuid;
use vm_fake::{FakeProvider, PrivateHttpResponder};
use vm_trait::{
    GuestCommand, NetworkMode, PrivateHttpRequest, PrivateHttpResponse, PrivateMailboxPublication,
    RootFilesystem, VmError, VmInstance, VmProvider, VmResources, VmSpec,
};
pub(super) fn limits() -> GatewayLimits {
    GatewayLimits {
        max_request_body_bytes: 8,
        max_response_body_bytes: 8,
        max_request_headers: 4,
        max_response_headers: 4,
        max_path_and_query_bytes: 64,
        execution_timeout: Duration::from_millis(50),
    }
}
pub(super) fn route() -> GatewayRouteBinding {
    GatewayRouteBinding {
        route_id: Uuid::new_v4(),
        exposure: gateway_domain::Exposure::Public,
        gateway_revision_id: Uuid::new_v4(),
        path_prefix: "echo".to_owned(),
        methods: BTreeSet::from([Method::POST]),
        limits: limits(),
    }
}
pub(super) fn caddy_template() -> LocalCaddyConfigurationTemplate {
    LocalCaddyConfigurationTemplate::new(
        serde_json::json!({
            "apps": { "http": { "servers": { "shared": {
                "listen": ["127.0.0.1:443"],
                "routes": [
                    { "match": [{ "path": ["/platform/*"] }], "handle": [{ "handler": "static_response", "body": "platform" }] },
                    { "group": "hephaestus.gateway", "handle": [{ "handler": "subroute", "routes": [] }] },
                    { "handle": [{ "handler": "static_response", "status_code": 404 }] }
                ]
            } } } }
        })
        .to_string()
        .as_bytes(),
        String::from("shared"),
    )
    .expect("valid shared Caddy template")
}
pub(super) fn caddy_template_with_ui_slot() -> LocalCaddyConfigurationTemplate {
    LocalCaddyConfigurationTemplate::new(
        serde_json::json!({
            "apps": { "http": { "servers": { "shared": {
                "listen": ["127.0.0.1:443"],
                "routes": [
                    { "group": "hephaestus.ui", "handle": [{ "handler": "subroute", "routes": [] }] },
                    { "match": [{ "path": ["/platform/*"] }], "handle": [{ "handler": "static_response", "body": "platform" }] },
                    { "group": "hephaestus.gateway", "handle": [{ "handler": "subroute", "routes": [] }] },
                    { "handle": [{ "handler": "static_response", "status_code": 404 }] }
                ]
            } } } }
        })
        .to_string()
        .as_bytes(),
        String::from("shared"),
    )
    .expect("valid shared Caddy template with UI slot")
}
pub(super) fn request(path: &str) -> GatewayRequest {
    GatewayRequest {
        method: Method::POST,
        path_and_query: path.to_owned(),
        headers: HeaderMap::new(),
        body: Bytes::from_static(b"ok"),
        trusted: TrustedRequestMetadata {
            scheme: GatewayScheme::Https,
            authority: "heph.test".to_owned(),
            client_address: "127.0.0.1".parse().expect("address"),
            request_id: Uuid::new_v4(),
        },
    }
}
pub(super) struct Admin(Mutex<Vec<Vec<u8>>>);
#[async_trait]
impl CaddyAdministration for Admin {
    async fn load(&self, config: Vec<u8>) -> Result<(), GatewayEdgeError> {
        self.0.lock().expect("lock").push(config);
        Ok(())
    }
}
pub(super) struct FailingAdmin;
#[async_trait]
impl CaddyAdministration for FailingAdmin {
    async fn load(&self, _: Vec<u8>) -> Result<(), GatewayEdgeError> {
        Err(GatewayEdgeError::Unavailable)
    }
}
pub(super) struct Resolver(GatewayRouteBinding);
#[async_trait]
impl GatewayRouteResolver for Resolver {
    async fn resolve(&self, _: &str) -> Result<Option<GatewayRouteBinding>, GatewayEdgeError> {
        Ok(Some(self.0.clone()))
    }
}
pub(super) struct Handler {
    calls: AtomicUsize,
    delay: Duration,
}
pub(super) struct CountingHandler(Arc<AtomicUsize>);
#[async_trait]
impl GatewayVmHandler for CountingHandler {
    async fn invoke(
        &self,
        _: &GatewayRouteBinding,
        _: Uuid,
        _: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(empty_response(StatusCode::CREATED))
    }
}

#[async_trait]
impl GatewayVmHandler for Handler {
    async fn invoke(
        &self,
        _: &GatewayRouteBinding,
        _: Uuid,
        _: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        Ok(empty_response(StatusCode::CREATED))
    }
}
pub(super) struct Recorder;
#[async_trait]
impl GatewayInvocationRecorder for Recorder {
    async fn accepted(&self, _: &GatewayRouteBinding, _: Uuid) -> Result<Uuid, GatewayEdgeError> {
        Ok(Uuid::new_v4())
    }
    async fn completed(
        &self,
        _: Uuid,
        _: GatewayInvocationOutcome,
    ) -> Result<(), GatewayEdgeError> {
        Ok(())
    }
}

pub(super) struct CompletionFailureRecorder {
    completions: AtomicUsize,
}
#[async_trait]
impl GatewayInvocationRecorder for CompletionFailureRecorder {
    async fn accepted(&self, _: &GatewayRouteBinding, _: Uuid) -> Result<Uuid, GatewayEdgeError> {
        Ok(Uuid::new_v4())
    }

    async fn accepted_ui(
        &self,
        _: &GatewayRouteBinding,
        _: &UiGatewayAuthority,
        _: Uuid,
    ) -> Result<Uuid, GatewayEdgeError> {
        Ok(Uuid::new_v4())
    }

    async fn completed(
        &self,
        _: Uuid,
        _: GatewayInvocationOutcome,
    ) -> Result<(), GatewayEdgeError> {
        self.completions.fetch_add(1, Ordering::SeqCst);
        Err(GatewayEdgeError::Unavailable)
    }
}

pub(super) struct EchoHandler;
#[async_trait]
impl GatewayVmHandler for EchoHandler {
    async fn invoke(
        &self,
        _: &GatewayRouteBinding,
        _: Uuid,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", HeaderValue::from_static("text/plain"));
        headers.insert("x-gateway-request-id", HeaderValue::from_static("trusted"));
        Ok(GatewayResponse {
            status: StatusCode::ACCEPTED,
            headers,
            body: request.body,
            mailbox_publication: None,
        })
    }
}

pub(super) struct AcceptMailbox;

#[async_trait]
impl GatewayMailboxPublisher for AcceptMailbox {
    async fn publish(
        &self,
        _: Uuid,
        publication: PrivateMailboxPublication,
    ) -> Result<(), GatewayEdgeError> {
        assert_eq!(publication.slot, "recipe-events");
        assert_eq!(publication.deduplication_key, "update-42");
        Ok(())
    }
}

pub(super) struct CountingMailbox(AtomicUsize);
#[async_trait]
impl GatewayMailboxPublisher for CountingMailbox {
    async fn publish(&self, _: Uuid, _: PrivateMailboxPublication) -> Result<(), GatewayEdgeError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub(super) struct InboundRules;
#[async_trait]
impl GatewayInboundSecretResolver for InboundRules {
    async fn rules_for_invocation(
        &self,
        _: Uuid,
        _: Uuid,
        _: Uuid,
    ) -> Result<Vec<InboundGatewaySecretRule>, gateway_domain::GatewayError> {
        Ok(vec![InboundGatewaySecretRule::new(
            HeaderName::from_static("x-hook-secret"),
            b"gateway-secret-sentinel".to_vec(),
            http::HeaderValue::from_static("heph-placeholder:v1:gateway-proof"),
        )?])
    }
}

pub(super) struct HeaderCapture(Mutex<Option<http::HeaderValue>>);
#[async_trait]
impl GatewayVmHandler for HeaderCapture {
    async fn invoke(
        &self,
        _: &GatewayRouteBinding,
        _: Uuid,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        *self.0.lock().expect("capture") = request.headers.get("x-hook-secret").cloned();
        Ok(empty_response(StatusCode::NO_CONTENT))
    }
}

pub(super) struct PrivateEcho;
#[async_trait]
impl PrivateHttpResponder for PrivateEcho {
    async fn invoke(&self, request: PrivateHttpRequest) -> Result<PrivateHttpResponse, VmError> {
        Ok(PrivateHttpResponse {
            status: StatusCode::CREATED,
            headers: HeaderMap::new(),
            body: request.body,
            mailbox_publication: Some(PrivateMailboxPublication {
                slot: "recipe-events".to_owned(),
                method: "POST".to_owned(),
                route: "/recipe".to_owned(),
                headers: vec![("source".to_owned(), "gateway".to_owned())],
                content_type: Some("application/json".to_owned()),
                trace_context: None,
                body: Bytes::from_static(b"event"),
                deduplication_key: "update-42".to_owned(),
            }),
        })
    }
}

pub(super) struct SlowPrivateHandler;
#[async_trait]
impl PrivateHttpResponder for SlowPrivateHandler {
    async fn invoke(&self, _: PrivateHttpRequest) -> Result<PrivateHttpResponse, VmError> {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(PrivateHttpResponse {
            status: StatusCode::NO_CONTENT,
            headers: HeaderMap::new(),
            body: Bytes::new(),
            mailbox_publication: None,
        })
    }
}

pub(super) struct FakeGatewayLauncher {
    provider: FakeProvider,
}
#[async_trait]
impl GatewayVmLauncher for FakeGatewayLauncher {
    async fn launch(
        &self,
        route: &GatewayRouteBinding,
        _: Uuid,
    ) -> Result<Arc<dyn VmInstance>, GatewayEdgeError> {
        self.provider
            .provision(gateway_vm_spec(route))
            .await
            .map_err(|_| GatewayEdgeError::HandlerUnavailable)
    }
}

pub(super) fn gateway_vm_spec(route: &GatewayRouteBinding) -> VmSpec {
    VmSpec {
        id: vm_trait::VmId(format!("gateway-{}", route.route_id)),
        root: RootFilesystem::Directory {
            host_path: "/gateway/release-root".into(),
        },
        disks: Vec::new(),
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 64,
        },
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: "/gateway/handler".to_owned(),
            args: Vec::new(),
            env: BTreeMap::new(),
            working_dir: None,
        },
        runtime_authority: None,
        runtime_git_bridge: None,
        private_http_service: None,
        labels: BTreeMap::new(),
    }
}
#[cfg(test)]
#[path = "caddy.rs"]
mod caddy;
#[cfg(test)]
#[path = "dispatch.rs"]
mod dispatch;
#[cfg(test)]
#[path = "dispatch_timeout.rs"]
mod dispatch_timeout;
#[cfg(test)]
#[path = "ui.rs"]
mod ui;
