use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, Path as AxumPath, State},
    http::{Request, StatusCode},
    response::Response,
    routing::{get, post},
};
use git_http::AuthenticatedHumanGitEndpoint;
use release_service::UiRepositoryGitOperation;
use std::{net::SocketAddr, sync::Arc};

use super::{UiRepositoryGitState, common::GitRoute, handlers, handlers::reject};
use crate::ui_audit::correlation_id;

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
    handlers::handle(
        peer,
        state,
        GitRoute {
            repository,
            endpoint,
            authority_operation: super::validation::authority_operation_for(endpoint),
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
    handlers::handle(
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
    handlers::handle(
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
