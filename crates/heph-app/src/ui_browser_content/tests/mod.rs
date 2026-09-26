pub(super) use super::gateway_policy::GatewayAuditDisposition;
pub(super) use super::gateway_response::sanitize_guest_response_headers;
pub(super) use super::static_content::parse_single_range;
pub(super) use super::{
    DEFAULT_MAX_STATIC_BYTES, UiContentError, UiContentState, UiGatewayDispatchAuthority,
    UiGatewayDispatchResult, UiGatewayDispatcher, UiGatewayRequestProjection, UiGatewayResponse,
    UiHttpRequest, UiNamespace, UiOriginConfig, UiPublicPort, apply_platform_csp,
    gateway_audit_disposition, gateway_response, hex_bytes, parse_child_cookie, platform_csp_value,
    redirect_response, require_unsafe_origin, router,
};
use async_trait::async_trait;
pub(super) use axum::{
    body::{Body, Bytes, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
};
pub(super) use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
pub(super) use gateway_domain::HttpMethod;
pub(super) use identity_domain::RequestId;
pub(super) use release_artifact_store::LocalArtifactStore;
pub(super) use release_domain::ui_browser::UiBrowserSessionSecret;
pub(super) use release_domain::{
    ContentHash, ReleaseArtifactId, UiInstallationGenerationId, UiInstallationId,
    ui::{UiCachePolicy, UiMediaType},
    ui_browser::{UiBrowserRoute, UiBrowserSessionId},
};
pub(super) use release_service::UiBrowserSessionContext;
pub(super) use release_service::ui_browser_host::{UI_CHILD_COOKIE, UiGenerationHost};
pub(super) use release_service::ui_browser_serving::{
    ActiveUiGenerationHost, UiBrowserHttpRequest, UiBrowserHttpServingProjection,
    UiGenerationHostResolver, UiHostLookupError, UiServingError, UiServingProjection,
    UiStaticArtifactProjection,
};
pub(super) use std::{
    fs,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::Arc,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
pub(super) use time::OffsetDateTime;
pub(super) use tokio::sync::Mutex;
pub(super) use tower::ServiceExt;
pub(super) use uuid::Uuid;

mod basic;
mod gateway;
mod static_content;

#[derive(Clone)]
struct FakeHostResolver {
    generation_id: UiInstallationGenerationId,
}

#[async_trait]
impl UiGenerationHostResolver for FakeHostResolver {
    async fn resolve_active_generation_host(
        &self,
        host: UiGenerationHost,
    ) -> Result<Option<ActiveUiGenerationHost>, UiHostLookupError> {
        Ok(
            (host.generation_id() == self.generation_id).then_some(ActiveUiGenerationHost {
                generation_id: self.generation_id,
            }),
        )
    }
}

struct FakeAuthority {
    calls: AtomicUsize,
    projection: Mutex<Option<UiServingProjection>>,
}

#[async_trait]
impl UiBrowserHttpServingProjection for FakeAuthority {
    async fn authenticate_and_project_http(
        &self,
        _request_id: RequestId,
        _session_secret: UiBrowserSessionSecret,
        _expected_generation_id: UiInstallationGenerationId,
        _request: UiBrowserHttpRequest,
    ) -> Result<UiServingProjection, UiServingError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.projection
            .lock()
            .await
            .clone()
            .ok_or(UiServingError::Unauthenticated)
    }
}

struct FakeGateway {
    calls: AtomicUsize,
    captured: Mutex<Option<UiGatewayDispatchAuthority>>,
    response: Mutex<Option<UiGatewayResponse>>,
}

struct StartedTimeoutGateway {
    started: Arc<AtomicBool>,
}

#[async_trait]
impl UiGatewayDispatcher for FakeGateway {
    async fn dispatch_ui_detailed(
        &self,
        authority: UiGatewayDispatchAuthority,
        _method: HttpMethod,
        _path_and_query: String,
        headers: HeaderMap,
        _body: Bytes,
    ) -> Result<UiGatewayDispatchResult, UiContentError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(headers.get(header::AUTHORIZATION).is_none());
        assert!(headers.get(header::COOKIE).is_none());
        *self.captured.lock().await = Some(authority);
        let response = self
            .response
            .lock()
            .await
            .clone()
            .ok_or(UiContentError::Unavailable)?;
        Ok(UiGatewayDispatchResult {
            response,
            disposition: gateway_edge::UiDispatchDisposition::Admitted {
                outcome: gateway_edge::GatewayInvocationOutcome::Completed,
                completion_persisted: true,
            },
        })
    }
}

#[async_trait]
impl UiGatewayDispatcher for StartedTimeoutGateway {
    async fn dispatch_ui_detailed(
        &self,
        _authority: UiGatewayDispatchAuthority,
        _method: HttpMethod,
        _path_and_query: String,
        _headers: HeaderMap,
        _body: Bytes,
    ) -> Result<UiGatewayDispatchResult, UiContentError> {
        self.started.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(250)).await;
        Err(UiContentError::Unavailable)
    }
}

fn context(generation_id: UiInstallationGenerationId) -> UiBrowserSessionContext {
    UiBrowserSessionContext {
        session_id: UiBrowserSessionId::from_uuid(Uuid::new_v4()),
        parent_session_id: identity_domain::BrowserSessionId::from_uuid(Uuid::new_v4()),
        actor_id: identity_domain::UserId::from_uuid(Uuid::new_v4()),
        organization_id: forge_domain::OrganizationId::from_uuid(Uuid::new_v4()),
        installation_id: UiInstallationId::from_uuid(Uuid::new_v4()),
        generation_id,
        route: UiBrowserRoute::parse("schema-ui").expect("route"),
        expires_at: OffsetDateTime::now_utc() + time::Duration::hours(1),
    }
}

fn host_and_state_with_sink(
    authority: Arc<FakeAuthority>,
    artifacts: Arc<LocalArtifactStore>,
    gateway: Arc<dyn UiGatewayDispatcher>,
    generation_id: UiInstallationGenerationId,
    audit_sink: Arc<dyn release_service::UiRequestAuditSink>,
) -> (Arc<UiContentState>, String) {
    let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
    let port = UiPublicPort::https_default();
    let host = UiGenerationHost::from_generation_id(generation_id);
    let state = UiContentState::new(
        Arc::new(FakeHostResolver { generation_id }),
        authority,
        artifacts,
        gateway,
        namespace,
        port,
        "https://app.example",
        audit_sink,
    )
    .expect("state");
    (
        Arc::new(state),
        host.authority(&UiNamespace::parse("ui.app.example").unwrap(), port),
    )
}

fn host_and_state(
    authority: Arc<FakeAuthority>,
    artifacts: Arc<LocalArtifactStore>,
    gateway: Arc<FakeGateway>,
    generation_id: UiInstallationGenerationId,
) -> (Arc<UiContentState>, String) {
    host_and_state_with_sink(
        authority,
        artifacts,
        gateway,
        generation_id,
        Arc::new(crate::ui_audit::NoopAuditSink),
    )
}

fn static_artifact(bytes: &[u8], storage_key: Uuid) -> UiStaticArtifactProjection {
    UiStaticArtifactProjection {
        artifact_id: ReleaseArtifactId::from_uuid(Uuid::new_v4()),
        storage_key,
        content_hash: ContentHash::digest(bytes),
        size_bytes: bytes.len() as u64,
        media_type: UiMediaType::TextPlain,
        cache_policy: UiCachePolicy::NoStore,
    }
}

fn temp_store(bytes: &[u8], storage_key: Uuid) -> (Arc<LocalArtifactStore>, PathBuf) {
    let root = std::env::temp_dir().join(format!("heph-ui-handler-{}", Uuid::new_v4()));
    fs::create_dir(&root).expect("artifact root");
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("root mode");
    fs::write(root.join(storage_key.simple().to_string()), bytes).expect("artifact");
    (
        Arc::new(LocalArtifactStore::new(root.clone()).expect("store")),
        root,
    )
}

fn content_request(
    method: &str,
    uri: &str,
    authority: &str,
    headers: &[(&str, &str)],
    body: Body,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, authority)
        .header(
            header::COOKIE,
            format!("{UI_CHILD_COOKIE}={}", URL_SAFE_NO_PAD.encode([7_u8; 32])),
        );
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let mut request = builder.body(body).expect("request");
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
    request
}
