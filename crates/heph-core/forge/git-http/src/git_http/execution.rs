use crate::{
    AuthenticatedHumanGitRequest, GitHttpService, GitOperation, Principal,
    errors::{authentication_error_response, error_response},
    principal::execute_principal,
};
use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
};
use forge_domain::RepositoryId;
use forge_service::GitStorage;
use identity_domain::RequestId;
use std::sync::Arc;
use zeroize::Zeroize;

// Keep route parsing, credential cleanup, and principal handoff together so
// authentication cannot accidentally bypass the common execution boundary.
#[allow(clippy::too_many_lines)]
pub async fn execute(
    service: Arc<GitHttpService>,
    route: String,
    operation: GitOperation,
    endpoint: &'static str,
    query: Option<String>,
    mut request: Request<Body>,
    receive: bool,
) -> Response<Body> {
    let repository_id = match GitStorage::parse_route(&route) {
        Ok(id) => id,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    };
    let mut credential = request
        .headers_mut()
        .remove(http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok().map(str::to_owned));
    let principal = service
        .authenticator
        .authenticate_git(
            credential.as_deref(),
            RequestId::new(),
            repository_id,
            operation,
        )
        .await;
    if let Some(credential) = &mut credential {
        credential.zeroize();
    }
    let principal = match principal {
        Ok(principal) => principal,
        Err(error) => return authentication_error_response(&error.to_string()),
    };
    execute_principal(
        service,
        GitExecutionRequest {
            repository_id,
            operation,
            endpoint,
            query,
            principal,
            request,
            receive,
        },
    )
    .await
}

/// Executes one Git transaction for a human identity already verified by an
/// enclosing trusted boundary.
///
/// The public Git router never calls this path: it always authenticates its
/// own bearer credential first. The call still performs Git's live repository
/// authorization before invoking the backend.
pub async fn execute_authenticated_human(
    service: Arc<GitHttpService>,
    authenticated: AuthenticatedHumanGitRequest,
) -> Response<Body> {
    let AuthenticatedHumanGitRequest {
        repository_id,
        endpoint,
        mut request,
        identity,
    } = authenticated;
    let (operation, endpoint_name, query, receive) = endpoint.parameters();
    // Browser session credentials are consumed by the UI authority boundary;
    // neither they nor a caller-supplied bearer token may reach Git.
    request.headers_mut().remove(http::header::AUTHORIZATION);
    request.headers_mut().remove(http::header::COOKIE);
    let principal = Principal::human(identity);
    execute_principal(
        service,
        GitExecutionRequest {
            repository_id,
            operation,
            endpoint: endpoint_name,
            query,
            principal,
            request,
            receive,
        },
    )
    .await
}

pub struct GitExecutionRequest {
    pub repository_id: RepositoryId,
    pub operation: GitOperation,
    pub endpoint: &'static str,
    pub query: Option<String>,
    pub principal: Principal,
    pub request: Request<Body>,
    pub receive: bool,
}
