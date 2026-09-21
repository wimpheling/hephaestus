//! Same-origin browser Git adapter for one installed UI repository.
//!
//! The adapter consumes the host-only child cookie at the UI boundary and
//! forwards only a verified human identity to the shared Git transport.  It
//! never forwards browser cookies or bearer headers to `git-http`.

use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, Path as AxumPath, State},
    http::{HeaderValue, Request, StatusCode, header},
    response::Response,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use forge_domain::RepositoryId;
use git_http::{
    AuthenticatedHumanGitEndpoint, AuthenticatedHumanGitRequest, GitHttpService,
    execute_authenticated_human,
};
use identity_domain::{AuthenticatedIdentity, RequestId};
use release_domain::ui_browser::UiBrowserSessionSecret;
use release_service::{
    ActiveUiGenerationHost, UiBrowserRepositoryGitAuthorization, UiGenerationHostResolver,
    UiGitAuthorizationError, UiHostLookupError, UiNamespace, UiPublicPort,
    UiRepositoryGitOperation,
    ui_browser_host::{UI_CHILD_COOKIE, UiGenerationHost},
};
use std::{net::SocketAddr, sync::Arc, time::Duration};

use crate::ui_audit::{UiAuditRecorder, correlation_id};

const MAX_QUERY_BYTES: usize = 256;
const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_DEADLINE: Duration = Duration::from_secs(15 * 60);
const GIT_ACTOR_HEADER: &str = "heph-git-actor-id";

/// State for the reserved UI-origin Git routes.
pub struct UiRepositoryGitState {
    host_resolver: Arc<dyn UiGenerationHostResolver>,
    authority: Arc<dyn UiBrowserRepositoryGitAuthorization>,
    git: Arc<GitHttpService>,
    namespace: UiNamespace,
    public_port: UiPublicPort,
    audit: UiAuditRecorder,
    deadline: Duration,
}

impl UiRepositoryGitState {
    /// Builds a bounded adapter around the existing live authority seams.
    pub fn new(
        host_resolver: Arc<dyn UiGenerationHostResolver>,
        authority: Arc<dyn UiBrowserRepositoryGitAuthorization>,
        git: Arc<GitHttpService>,
        namespace: UiNamespace,
        public_port: UiPublicPort,
        audit_sink: Arc<dyn release_service::UiRequestAuditSink>,
    ) -> Self {
        Self {
            host_resolver,
            authority,
            git,
            namespace,
            public_port,
            audit: UiAuditRecorder::new(audit_sink),
            deadline: MAX_DEADLINE,
        }
    }
}

/// Registers only the three reserved smart-HTTP endpoints.
pub fn router(state: Arc<UiRepositoryGitState>) -> Router {
    Router::new()
        .route("/_heph/git/{repository}/info/refs", get(info_refs))
        .route("/_heph/git/{repository}/git-upload-pack", post(upload_pack))
        .route(
            "/_heph/git/{repository}/git-receive-pack",
            post(receive_pack),
        )
        .with_state(state)
}

async fn info_refs(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<UiRepositoryGitState>>,
    AxumPath(repository): AxumPath<String>,
    request: Request<Body>,
) -> Response<Body> {
    let request_id = correlation_id(request.extensions());
    let service = match request.uri().query() {
        Some("service=git-upload-pack") => "git-upload-pack",
        Some("service=git-receive-pack") => "git-receive-pack",
        _ => {
            return reject(
                &state,
                request_id,
                StatusCode::BAD_REQUEST,
                "ui_git_invalid_query",
            )
            .await;
        }
    };
    let endpoint = if service == "git-upload-pack" {
        AuthenticatedHumanGitEndpoint::CloneInfoRefs
    } else {
        AuthenticatedHumanGitEndpoint::PushInfoRefs
    };
    handle(
        peer,
        state,
        GitRoute {
            repository,
            endpoint,
            authority_operation: authority_operation_for(endpoint),
        },
        request,
    )
    .await
}

async fn upload_pack(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<UiRepositoryGitState>>,
    AxumPath(repository): AxumPath<String>,
    request: Request<Body>,
) -> Response<Body> {
    handle(
        peer,
        state,
        GitRoute {
            repository,
            endpoint: AuthenticatedHumanGitEndpoint::UploadPack,
            authority_operation: UiRepositoryGitOperation::Read,
        },
        request,
    )
    .await
}

async fn receive_pack(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<UiRepositoryGitState>>,
    AxumPath(repository): AxumPath<String>,
    request: Request<Body>,
) -> Response<Body> {
    handle(
        peer,
        state,
        GitRoute {
            repository,
            endpoint: AuthenticatedHumanGitEndpoint::ReceivePack,
            authority_operation: UiRepositoryGitOperation::Write,
        },
        request,
    )
    .await
}

// Keep the security gates visibly ordered in one adapter: loopback, bounded
// syntax, canonical host, origin, live authority, then Git execution.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
async fn handle(
    peer: SocketAddr,
    state: Arc<UiRepositoryGitState>,
    route: GitRoute,
    mut request: Request<Body>,
) -> Response<Body> {
    let request_id = correlation_id(request.extensions());
    if !peer.ip().is_loopback() {
        return state
            .audit
            .denial(
                request_id,
                release_service::UiRequestAuditSurface::Content,
                release_service::UiRequestAuditReason::Unauthorized,
                error_response(StatusCode::FORBIDDEN, "ui_forbidden"),
            )
            .await;
    }
    let header_bytes = request
        .headers()
        .iter()
        .map(|(name, value)| name.as_str().len().saturating_add(value.as_bytes().len()))
        .sum::<usize>();
    if header_bytes > MAX_HEADER_BYTES {
        return reject(
            &state,
            request_id,
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            "ui_git_headers",
        )
        .await;
    }
    let repository_id = match route.repository.parse::<uuid::Uuid>() {
        Ok(uuid) if uuid.to_string() == route.repository => RepositoryId::from_uuid(uuid),
        Ok(_) | Err(_) => {
            return reject(
                &state,
                request_id,
                StatusCode::BAD_REQUEST,
                "ui_git_repository",
            )
            .await;
        }
    };
    if request
        .uri()
        .query()
        .is_some_and(|value| value.len() > MAX_QUERY_BYTES)
        || (route.endpoint.backend_endpoint() != "info/refs" && request.uri().query().is_some())
    {
        return reject(&state, request_id, StatusCode::URI_TOO_LONG, "ui_git_query").await;
    }
    let authority = match request_authority(&request) {
        Ok(authority) => authority,
        Err(response) => return reject(&state, request_id, response, "ui_git_authority").await,
    };
    let Ok(host) = UiGenerationHost::parse(&authority, &state.namespace, state.public_port) else {
        return reject(
            &state,
            request_id,
            StatusCode::NOT_FOUND,
            "ui_git_not_found",
        )
        .await;
    };
    match state
        .host_resolver
        .resolve_active_generation_host(host)
        .await
    {
        Ok(Some(ActiveUiGenerationHost { generation_id }))
            if generation_id == host.generation_id() => {}
        Ok(Some(_) | None) => {
            return reject(
                &state,
                request_id,
                StatusCode::NOT_FOUND,
                "ui_git_not_found",
            )
            .await;
        }
        Err(UiHostLookupError::Unavailable) => {
            return reject(
                &state,
                request_id,
                StatusCode::SERVICE_UNAVAILABLE,
                "ui_unavailable",
            )
            .await;
        }
    }
    if let Err(status) = require_same_origin(
        &request,
        host,
        &state.namespace,
        state.public_port,
        route.endpoint.backend_endpoint() != "info/refs",
    ) {
        return reject(&state, request_id, status, "ui_git_origin").await;
    }
    if let Some(length) = request.headers().get(header::CONTENT_LENGTH) {
        let Ok(length) = length
            .to_str()
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or(())
        else {
            return reject(&state, request_id, StatusCode::BAD_REQUEST, "ui_git_length").await;
        };
        if length > state.git.limits().max_request_bytes {
            return reject(
                &state,
                request_id,
                StatusCode::PAYLOAD_TOO_LARGE,
                "ui_git_body",
            )
            .await;
        }
    }
    let secret = match parse_child_cookie(&request) {
        Ok(secret) => secret,
        Err(status) => return reject(&state, request_id, status, "ui_git_cookie").await,
    };
    let authorization = match state
        .authority
        .authorize_repository_git(
            request_id,
            secret,
            host.generation_id(),
            repository_id,
            route.authority_operation,
        )
        .await
    {
        Ok(authorization)
            if authorization.repository_id == repository_id
                && match route.authority_operation {
                    UiRepositoryGitOperation::Read => !matches!(
                        authorization.access,
                        release_domain::ui::UiRepositoryGitAccess::None
                    ),
                    UiRepositoryGitOperation::Write => matches!(
                        authorization.access,
                        release_domain::ui::UiRepositoryGitAccess::ReadWrite
                    ),
                } =>
        {
            authorization
        }
        Ok(_) | Err(UiGitAuthorizationError::Unauthorized) => {
            return reject(
                &state,
                request_id,
                StatusCode::FORBIDDEN,
                "ui_git_forbidden",
            )
            .await;
        }
        Err(UiGitAuthorizationError::Unavailable) => {
            return reject(
                &state,
                request_id,
                StatusCode::SERVICE_UNAVAILABLE,
                "ui_unavailable",
            )
            .await;
        }
    };
    // The UI cookie and any browser bearer header end at this boundary.
    request.headers_mut().remove(header::COOKIE);
    request.headers_mut().remove(header::AUTHORIZATION);
    let identity = AuthenticatedIdentity::new(
        authorization.actor_id,
        "urn:hephaestus:ui-browser",
        format!("user:{}", authorization.actor_id),
        serde_json::Value::Object(serde_json::Map::new()),
        request_id,
    );
    let response = tokio::time::timeout(
        state.deadline,
        execute_authenticated_human(
            Arc::clone(&state.git),
            AuthenticatedHumanGitRequest {
                repository_id,
                endpoint: route.endpoint,
                request,
                identity,
            },
        ),
    )
    .await;
    match response {
        Ok(response) if response.status().is_success() => {
            let response = actor_header(response, authorization.actor_id, route.endpoint);
            state
                .audit
                .allowed(
                    request_id,
                    release_service::UiRequestAuditSurface::Content,
                    crate::ui_audit::verified_context(&authorization.context),
                    release_service::UiRequestAuditOutcome::Succeeded,
                    release_service::UiRequestAuditReason::None,
                    response,
                )
                .await
        }
        Ok(response) => {
            state
                .audit
                .failed(
                    request_id,
                    release_service::UiRequestAuditSurface::Content,
                    crate::ui_audit::verified_context(&authorization.context),
                    release_service::UiRequestAuditReason::UpstreamFailure,
                    response,
                )
                .await
        }
        Err(_) => {
            state
                .audit
                .undetermined(
                    request_id,
                    release_service::UiRequestAuditSurface::Content,
                    crate::ui_audit::verified_context(&authorization.context),
                    error_response(StatusCode::SERVICE_UNAVAILABLE, "ui_unavailable"),
                )
                .await
        }
    }
}

fn actor_header(
    mut response: Response<Body>,
    actor_id: identity_domain::UserId,
    endpoint: AuthenticatedHumanGitEndpoint,
) -> Response<Body> {
    if endpoint.backend_endpoint() == "info/refs" {
        if let Ok(value) = HeaderValue::from_str(&actor_id.to_string()) {
            response.headers_mut().insert(GIT_ACTOR_HEADER, value);
        }
    }
    response
}

struct GitRoute {
    repository: String,
    endpoint: AuthenticatedHumanGitEndpoint,
    authority_operation: UiRepositoryGitOperation,
}

async fn reject(
    state: &UiRepositoryGitState,
    request_id: RequestId,
    status: StatusCode,
    message: &'static str,
) -> Response<Body> {
    state
        .audit
        .denial(
            request_id,
            release_service::UiRequestAuditSurface::Content,
            if status == StatusCode::NOT_FOUND {
                release_service::UiRequestAuditReason::NotFound
            } else if status == StatusCode::SERVICE_UNAVAILABLE {
                release_service::UiRequestAuditReason::Unavailable
            } else {
                release_service::UiRequestAuditReason::InvalidInput
            },
            error_response(status, message),
        )
        .await
}

pub fn request_authority(request: &Request<Body>) -> Result<String, StatusCode> {
    if request.uri().scheme().is_some()
        || request.uri().authority().is_some()
        || request.headers().get_all(header::HOST).iter().count() != 1
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or(StatusCode::BAD_REQUEST)
}

pub fn require_same_origin(
    request: &Request<Body>,
    host: UiGenerationHost,
    namespace: &UiNamespace,
    public_port: UiPublicPort,
    unsafe_request: bool,
) -> Result<(), StatusCode> {
    let expected = format!("https://{}", host.authority(namespace, public_port));
    let origins = request.headers().get_all(header::ORIGIN);
    if origins.iter().count() == 0 && !unsafe_request {
        return Ok(());
    }
    if origins.iter().count() != 1
        || origins.iter().next().and_then(|value| value.to_str().ok()) != Some(expected.as_str())
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if unsafe_request
        && request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|value| value.as_bytes() != b"same-origin")
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(())
}

pub fn parse_child_cookie(request: &Request<Body>) -> Result<UiBrowserSessionSecret, StatusCode> {
    let mut found = None;
    for value in &request.headers().get_all(header::COOKIE) {
        let value = value.to_str().map_err(|_| StatusCode::UNAUTHORIZED)?;
        for pair in value.split(';') {
            let pair = pair.trim();
            let Some((name, encoded)) = pair.split_once('=') else {
                return Err(StatusCode::UNAUTHORIZED);
            };
            if name != UI_CHILD_COOKIE {
                continue;
            }
            if found.is_some()
                || encoded.len() != 43
                || !encoded
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            {
                return Err(StatusCode::UNAUTHORIZED);
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(encoded)
                .map_err(|_| StatusCode::UNAUTHORIZED)?;
            let bytes: [u8; 32] = bytes.try_into().map_err(|_| StatusCode::UNAUTHORIZED)?;
            found = Some(UiBrowserSessionSecret::from_bytes(bytes));
        }
    }
    found.ok_or(StatusCode::UNAUTHORIZED)
}

pub fn error_response(status: StatusCode, message: &'static str) -> Response<Body> {
    let mut response = Response::new(Body::from(format!("{{\"error\":\"{message}\"}}")));
    *response.status_mut() = status;
    response
}

const fn authority_operation_for(
    endpoint: AuthenticatedHumanGitEndpoint,
) -> UiRepositoryGitOperation {
    match endpoint {
        AuthenticatedHumanGitEndpoint::CloneInfoRefs
        | AuthenticatedHumanGitEndpoint::UploadPack => UiRepositoryGitOperation::Read,
        AuthenticatedHumanGitEndpoint::PushInfoRefs
        | AuthenticatedHumanGitEndpoint::ReceivePack => UiRepositoryGitOperation::Write,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use authz_domain::{
        AuthorizationDecision, AuthzError, GitRepositoryAuthorizer, GitRepositoryOperation,
    };
    use authz_postgres::PostgresMelangeAuthorizer;
    use axum::{body::Body, extract::ConnectInfo};
    use forge_postgres::PgForgeRepository;
    use forge_service::GitStorage;
    use git_http::{AuthenticationError, GitAuthenticator, PostgresGitAuthorizer, Principal};
    use release_service::{
        UiBrowserRepositoryGitAuthorization, UiGenerationHostResolver, UiGitAuthorizationError,
        UiHostLookupError, UiRepositoryGitAuthorization, UiRepositoryGitOperation,
    };
    #[allow(dead_code)]
    mod resource_fixture {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../release-postgres/tests/support/ui_browser_resource_fixture.rs"
        ));
    }
    use sqlx::postgres::PgPoolOptions;
    use std::{env, net::SocketAddr, path::PathBuf, process::Output};
    use tower::ServiceExt;
    use uuid::Uuid;

    #[test]
    fn same_origin_requires_exact_generation_origin_for_all_git_requests() {
        let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
        let port = UiPublicPort::https_default();
        let generation = release_domain::UiInstallationGenerationId::from_uuid(
            uuid::Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").expect("uuid"),
        );
        let host = UiGenerationHost::from_generation_id(generation);
        let request = Request::builder()
            .uri("/_heph/git/01234567-89ab-cdef-0123-456789abcdef/info/refs")
            .header(header::HOST, host.authority(&namespace, port))
            .header(
                header::ORIGIN,
                format!("https://{}", host.authority(&namespace, port)),
            )
            .body(Body::empty())
            .expect("request");
        assert!(require_same_origin(&request, host, &namespace, port, true).is_ok());
        let missing = Request::builder()
            .uri("/_heph/git/01234567-89ab-cdef-0123-456789abcdef/info/refs")
            .header(header::HOST, host.authority(&namespace, port))
            .body(Body::empty())
            .expect("request");
        assert!(require_same_origin(&missing, host, &namespace, port, false).is_ok());
        assert!(require_same_origin(&missing, host, &namespace, port, true).is_err());
    }

    #[test]
    fn child_cookie_parser_rejects_duplicates_and_accepts_only_32_bytes() {
        let encoded = URL_SAFE_NO_PAD.encode([7_u8; 32]);
        let request = Request::builder()
            .header(header::COOKIE, format!("{UI_CHILD_COOKIE}={encoded}"))
            .body(Body::empty())
            .expect("request");
        assert!(parse_child_cookie(&request).is_ok());
        let duplicate = Request::builder()
            .header(
                header::COOKIE,
                format!("{UI_CHILD_COOKIE}={encoded}; {UI_CHILD_COOKIE}={encoded}"),
            )
            .body(Body::empty())
            .expect("request");
        assert!(parse_child_cookie(&duplicate).is_err());
    }

    struct TestHostResolver {
        active_generation: Option<release_domain::UiInstallationGenerationId>,
    }

    #[async_trait]
    impl UiGenerationHostResolver for TestHostResolver {
        async fn resolve_active_generation_host(
            &self,
            host: UiGenerationHost,
        ) -> Result<Option<ActiveUiGenerationHost>, UiHostLookupError> {
            Ok(self
                .active_generation
                .filter(|generation| *generation == host.generation_id())
                .map(|generation_id| ActiveUiGenerationHost { generation_id }))
        }
    }

    struct TestGitAuthority;

    #[async_trait]
    impl UiBrowserRepositoryGitAuthorization for TestGitAuthority {
        async fn authorize_repository_git(
            &self,
            _request_id: RequestId,
            _session_secret: UiBrowserSessionSecret,
            _expected_generation_id: release_domain::UiInstallationGenerationId,
            _repository_id: RepositoryId,
            _operation: UiRepositoryGitOperation,
        ) -> Result<UiRepositoryGitAuthorization, UiGitAuthorizationError> {
            Err(UiGitAuthorizationError::Unauthorized)
        }
    }

    struct TestAuthenticator;

    #[async_trait]
    impl GitAuthenticator for TestAuthenticator {
        async fn authenticate(
            &self,
            _credential: Option<&str>,
            _request_id: RequestId,
        ) -> Result<Principal, AuthenticationError> {
            Err(AuthenticationError::denied("test"))
        }
    }

    struct AllowGit;

    #[async_trait]
    impl GitRepositoryAuthorizer for AllowGit {
        async fn authorize_git(
            &self,
            _repository_id: uuid::Uuid,
            _operation: GitRepositoryOperation,
            _identity: &identity_domain::AuthenticatedIdentity,
        ) -> Result<AuthorizationDecision, AuthzError> {
            Ok(AuthorizationDecision::Allow)
        }
    }

    async fn test_router() -> Router {
        test_router_with_host(None).await
    }

    async fn test_router_with_host(
        active_generation: Option<release_domain::UiInstallationGenerationId>,
    ) -> Router {
        let root = tempfile::tempdir().expect("Git root");
        let storage = Arc::new(GitStorage::initialize(root.path()).await.expect("storage"));
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://test:test@127.0.0.1:1/test")
            .expect("lazy pool");
        let repository = Arc::new(PgForgeRepository::new(pool, Arc::clone(&storage)));
        let authorizer = Arc::new(PostgresGitAuthorizer::new(Arc::new(AllowGit)));
        let git = Arc::new(
            GitHttpService::new(
                repository,
                storage,
                Arc::new(TestAuthenticator),
                authorizer,
                PathBuf::from("/bin/false"),
                Default::default(),
            )
            .expect("Git service"),
        );
        let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
        let state = Arc::new(UiRepositoryGitState::new(
            Arc::new(TestHostResolver { active_generation }),
            Arc::new(TestGitAuthority),
            git,
            namespace,
            UiPublicPort::https_default(),
            Arc::new(crate::ui_audit::NoopAuditSink),
        ));
        router(state)
    }

    #[tokio::test]
    async fn actual_router_constructs_and_routes_reserved_path() {
        let app = test_router().await;
        let mut request = Request::builder()
            .method("GET")
            .uri(format!(
                "/_heph/git/{}/info/refs?service=unsupported",
                RepositoryId::new()
            ))
            .header(
                header::HOST,
                "g-0123456789abcdef0123456789abcdef.ui.example.test",
            )
            .body(Body::empty())
            .expect("request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));
        let response = app.oneshot(request).await.expect("router response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn actual_router_rejects_missing_cookie_and_cross_origin_requests() {
        let generation = release_domain::UiInstallationGenerationId::from_uuid(
            uuid::Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").expect("uuid"),
        );
        let host = UiGenerationHost::from_generation_id(generation);
        let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
        let authority = host.authority(&namespace, UiPublicPort::https_default());
        let app = test_router_with_host(Some(generation)).await;
        let mut missing_cookie = Request::builder()
            .method("GET")
            .uri(format!(
                "/_heph/git/{}/info/refs?service=git-upload-pack",
                generation.as_uuid()
            ))
            .header(header::HOST, &authority)
            .body(Body::empty())
            .expect("missing-cookie request");
        missing_cookie
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));
        assert_eq!(
            app.oneshot(missing_cookie)
                .await
                .expect("missing-cookie response")
                .status(),
            StatusCode::UNAUTHORIZED
        );

        let app = test_router_with_host(Some(generation)).await;
        let encoded = URL_SAFE_NO_PAD.encode([7_u8; 32]);
        let mut cross_origin = Request::builder()
            .method("POST")
            .uri(format!(
                "/_heph/git/{}/git-upload-pack",
                generation.as_uuid()
            ))
            .header(header::HOST, &authority)
            .header(header::ORIGIN, "https://wrong.ui.example.test")
            .header(header::COOKIE, format!("{UI_CHILD_COOKIE}={encoded}"))
            .body(Body::empty())
            .expect("cross-origin request");
        cross_origin
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 1234))));
        assert_eq!(
            app.oneshot(cross_origin)
                .await
                .expect("cross-origin response")
                .status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn real_child_fetch_and_push_route_persists_human_receive_and_audit() {
        let Some(database_url) = env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
            eprintln!("skipping live UI Git route test: HEPHAESTUS_POSTGRES_TEST_URL is unset");
            return;
        };
        let bootstrap = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("connect bootstrap PostgreSQL");
        sqlx::migrate!("../../migrations")
            .run(&bootstrap)
            .await
            .expect("apply migrations through 0098");
        let worker = role_pool(&database_url, "hephaestus_worker").await;
        let app_pool = role_pool(&database_url, "hephaestus_app").await;
        let fixture = resource_fixture::seed_fixture_reusing_installation_helpers(&worker).await;
        let secret = resource_fixture::fixture_session_secret(fixture.actor, 77);
        // The shared UI fixture intentionally leaves repository write authority
        // absent so its revocation matrix remains meaningful. This production
        // route proof needs an explicit live human CanWrite grant.
        sqlx::query(
            "INSERT INTO project_maintainers (project_id, user_id)
             VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(fixture.source_project)
        .bind(fixture.actor)
        .execute(&worker)
        .await
        .expect("seed route-test repository maintainer");
        insert_repository_child(&worker, &fixture, secret).await;

        let temporary = tempfile::tempdir().expect("temporary Git root");
        let storage = Arc::new(
            GitStorage::initialize(temporary.path().join("repositories"))
                .await
                .expect("Git storage"),
        );
        let repository_id = RepositoryId::from_uuid(fixture.repository);
        storage
            .create_bare(repository_id, "main")
            .await
            .expect("create canonical bare repository");
        let source = temporary.path().join("source");
        run_git(
            &temporary.path(),
            &["init", "--initial-branch=main", source.to_str().unwrap()],
        )
        .await;
        run_git(&source, &["config", "user.name", "UI Test"]).await;
        run_git(
            &source,
            &["config", "user.email", "ui-test@example.invalid"],
        )
        .await;
        tokio::fs::write(source.join("README.md"), "initial\n")
            .await
            .expect("write initial source");
        run_git(&source, &["add", "."]).await;
        run_git(&source, &["commit", "-m", "initial"]).await;
        run_git(
            &source,
            &[
                "push",
                storage.repository_path(repository_id).to_str().unwrap(),
                "main",
            ],
        )
        .await;

        let backend = git_backend_path().await;
        let forge = Arc::new(
            PgForgeRepository::new(worker.clone(), Arc::clone(&storage))
                .with_authorizer(Arc::new(PostgresMelangeAuthorizer)),
        );
        let git = Arc::new(
            GitHttpService::new(
                forge,
                Arc::clone(&storage),
                Arc::new(TestAuthenticator),
                Arc::new(PostgresGitAuthorizer::new(Arc::new(
                    authz_postgres::PostgresGitAuthorizer::new(worker.clone()),
                ))),
                backend,
                Default::default(),
            )
            .expect("Git HTTP service"),
        );
        let namespace = UiNamespace::parse("ui.example.test").expect("namespace");
        let port = UiPublicPort::https_default();
        let host = UiGenerationHost::from_generation_id(
            release_domain::UiInstallationGenerationId::from_uuid(fixture.repository_generation),
        );
        let authority = host.authority(&namespace, port);
        let host_resolver: Arc<dyn UiGenerationHostResolver> = Arc::new(
            release_postgres::PgUiGenerationHostResolver::new(app_pool.clone()),
        );
        let serving_store = Arc::new(release_postgres::PgUiBrowserServingStore::new(
            app_pool.clone(),
        ));
        let audit_sink: Arc<dyn release_service::UiRequestAuditSink> = Arc::new(
            release_postgres::PgUiRequestAuditRepository::new(worker.clone()),
        );
        let state = Arc::new(UiRepositoryGitState::new(
            host_resolver.clone(),
            serving_store.clone(),
            Arc::clone(&git),
            namespace.clone(),
            port,
            audit_sink.clone(),
        ));
        let context = Arc::new(crate::ui_context::UiContextState::new(
            host_resolver,
            serving_store,
            namespace.clone(),
            port,
            audit_sink,
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind UI Git test listener");
        let address = listener.local_addr().expect("UI Git listener address");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                router(state)
                    .merge(crate::ui_context::router(context))
                    .into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .expect("UI Git server");
        });
        let remote = format!("http://{address}/_heph/git/{}", repository_id);
        let cookie = format!("{UI_CHILD_COOKIE}={}", URL_SAFE_NO_PAD.encode(secret));
        let context_result = tokio::process::Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--fail",
                "-H",
                &format!("Host: {authority}"),
                "-H",
                &format!("Cookie: {cookie}"),
                &format!("http://{address}/_heph/ui-context"),
            ])
            .output()
            .await
            .expect("spawn UI context curl");
        assert!(
            context_result.status.success(),
            "UI context request failed: {context_result:?}"
        );
        let context: serde_json::Value =
            serde_json::from_slice(&context_result.stdout).expect("UI context JSON");
        assert_eq!(context["repository_id"], fixture.repository.to_string());
        assert_eq!(context.as_object().expect("UI context object").len(), 1);
        let common = vec![
            String::from("-c"),
            format!("http.extraHeader=Host: {authority}"),
            String::from("-c"),
            format!("http.extraHeader=Cookie: {cookie}"),
            String::from("-c"),
            format!("http.extraHeader=Origin: https://{authority}"),
        ];
        let clone_path = temporary.path().join("clone");
        let mut clone_args = common.clone();
        clone_args.extend([
            String::from("clone"),
            remote.clone(),
            clone_path.to_str().unwrap().to_owned(),
        ]);
        let clone_result = run_git_owned(temporary.path(), clone_args).await;
        assert!(
            clone_result.status.success(),
            "UI Git clone failed: {clone_result:?}"
        );
        assert_eq!(
            tokio::fs::read_to_string(clone_path.join("README.md"))
                .await
                .expect("cloned README"),
            "initial\n"
        );
        run_git(&clone_path, &["config", "user.name", "UI Test"]).await;
        run_git(
            &clone_path,
            &["config", "user.email", "ui-test@example.invalid"],
        )
        .await;
        tokio::fs::write(clone_path.join("README.md"), "pushed\n")
            .await
            .expect("write pushed source");
        run_git(&clone_path, &["add", "."]).await;
        run_git(&clone_path, &["commit", "-m", "browser push"]).await;
        let mut push_args = common;
        push_args.extend([String::from("push"), remote, String::from("HEAD:main")]);
        let push_result = run_git_owned(&clone_path, push_args).await;
        run_git(
            storage
                .repository_path(repository_id)
                .parent()
                .expect("bare parent"),
            &[
                "--git-dir",
                storage.repository_path(repository_id).to_str().unwrap(),
                "rev-parse",
                "refs/heads/main",
            ],
        )
        .await;
        assert!(
            push_result.status.success(),
            "UI Git push failed: {push_result:?}"
        );
        let receive: (Uuid, Uuid, String, Uuid) = sqlx::query_as(
            "SELECT id, actor_id, principal, request_id
             FROM git_receives
             WHERE repository_id = $1 AND principal = $2
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(fixture.repository)
        .bind(format!("user:{}", fixture.actor))
        .fetch_one(&bootstrap)
        .await
        .expect("durable human receive");
        assert_eq!(receive.1, fixture.actor);
        assert_eq!(receive.2, format!("user:{}", fixture.actor));
        let audit: (String, String, Uuid, Uuid, Uuid) = sqlx::query_as(
            "SELECT decision, outcome, actor_id, installation_id, generation_id
             FROM ui_request_audit_events
             WHERE request_id = $1",
        )
        .bind(receive.3)
        .fetch_one(&bootstrap)
        .await
        .expect("durable UI Git audit event");
        assert_eq!(audit.0, "allowed");
        assert_eq!(audit.1, "succeeded");
        assert_eq!(audit.2, fixture.actor);
        assert_eq!(audit.3, fixture.repository_installation);
        assert_eq!(audit.4, fixture.repository_generation);
        server.abort();
    }

    async fn run_git(directory: &std::path::Path, args: &[&str]) {
        let result = tokio::process::Command::new("git")
            .args(args)
            .current_dir(directory)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .await
            .expect("spawn git");
        assert!(result.status.success(), "git failed: {result:?}");
    }

    async fn run_git_owned(directory: &std::path::Path, args: Vec<String>) -> Output {
        tokio::process::Command::new("git")
            .args(&args)
            .current_dir(directory)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .await
            .expect("spawn git")
    }

    async fn git_backend_path() -> PathBuf {
        let output = tokio::process::Command::new("git")
            .args(["--exec-path"])
            .output()
            .await
            .expect("git exec path");
        assert!(output.status.success());
        PathBuf::from(
            String::from_utf8(output.stdout)
                .expect("git exec path UTF-8")
                .trim(),
        )
        .join("git-http-backend")
    }

    async fn role_pool(database_url: &str, role: &str) -> sqlx::PgPool {
        PgPoolOptions::new()
            .max_connections(4)
            .after_connect({
                let role = role.to_owned();
                move |connection, _metadata| {
                    let role = role.clone();
                    Box::pin(async move {
                        sqlx::query("SELECT set_config('role', $1, false)")
                            .bind(role)
                            .execute(&mut *connection)
                            .await
                            .map(|_| ())
                    })
                }
            })
            .connect(database_url)
            .await
            .expect("connect role pool")
    }

    async fn insert_repository_child(
        pool: &sqlx::PgPool,
        fixture: &resource_fixture::Fixture,
        secret: [u8; 32],
    ) {
        let handoff = uuid::Uuid::new_v4();
        let mut transaction = pool.begin().await.expect("begin repository child");
        sqlx::query(
            "INSERT INTO ui_browser_handoffs
             (id, handoff_digest, request_id, actor_id, parent_session_id,
              installation_id, generation_id, organization_id, route,
              issued_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'schema-repository',
                     statement_timestamp(), statement_timestamp() + interval '60 seconds')",
        )
        .bind(handoff)
        .bind(secret.to_vec())
        .bind(uuid::Uuid::new_v4())
        .bind(fixture.actor)
        .bind(fixture.parent_session)
        .bind(fixture.repository_installation)
        .bind(fixture.repository_generation)
        .bind(fixture.organization)
        .execute(&mut *transaction)
        .await
        .expect("insert repository handoff");
        sqlx::query(
            "INSERT INTO ui_browser_sessions
             (id, session_digest, request_id, handoff_id, parent_session_id,
              installation_id, generation_id, organization_id, route,
              issued_at, expires_at)
             SELECT $1, $2, $3, id, parent_session_id, installation_id,
                    generation_id, organization_id, route, issued_at,
                    statement_timestamp() + interval '1 hour'
             FROM ui_browser_handoffs WHERE id = $4",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(
            UiBrowserSessionSecret::from_bytes(secret)
                .digest()
                .as_bytes()
                .to_vec(),
        )
        .bind(uuid::Uuid::new_v4())
        .bind(handoff)
        .execute(&mut *transaction)
        .await
        .expect("insert repository child");
        sqlx::query(
            "UPDATE ui_browser_handoffs SET consumed_at = statement_timestamp() WHERE id = $1",
        )
        .bind(handoff)
        .execute(&mut *transaction)
        .await
        .expect("consume repository handoff");
        transaction.commit().await.expect("commit repository child");
    }

    #[test]
    fn actor_header_is_only_added_to_discovery_responses() {
        let actor = identity_domain::UserId::new();
        let response = Response::new(Body::empty());
        let response = actor_header(
            response,
            actor,
            AuthenticatedHumanGitEndpoint::CloneInfoRefs,
        );
        assert_eq!(
            response
                .headers()
                .get(GIT_ACTOR_HEADER)
                .unwrap()
                .to_str()
                .unwrap(),
            actor.to_string()
        );
        let response = actor_header(
            Response::new(Body::empty()),
            actor,
            AuthenticatedHumanGitEndpoint::UploadPack,
        );
        assert!(response.headers().get(GIT_ACTOR_HEADER).is_none());
    }
}
