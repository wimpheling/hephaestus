#[path = "principal/stream.rs"]
mod stream;

use crate::{
    AuthorizationRequest, GitHttpService, Principal,
    errors::{error_response, service_error},
    execution::GitExecutionRequest,
    receive_policy,
};
use axum::{
    body::Body,
    http::{Response, StatusCode},
};
use forge_domain::RuntimeReceiveProvenance;
use forge_service::ForgeRepositoryError;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

// Keep the shared authorization, receive lock, streaming backend, and
// persistence lifecycle together so public and trusted human entry points
// cannot drift in security-critical ordering.
#[allow(clippy::too_many_lines)]
pub async fn execute_principal(
    service: Arc<GitHttpService>,
    execution: GitExecutionRequest,
) -> Response<Body> {
    let GitExecutionRequest {
        repository_id,
        operation,
        endpoint,
        query,
        principal,
        request,
        receive,
    } = execution;
    let runtime_receive_provenance = if receive {
        match &principal {
            Principal::Human(_) => None,
            Principal::Runtime(runtime) => {
                let Ok(runtime_session_id) = runtime.runtime_session_id().parse() else {
                    return error_response(
                        StatusCode::FORBIDDEN,
                        "runtime receive session identity is invalid",
                    );
                };
                Some(RuntimeReceiveProvenance { runtime_session_id })
            }
        }
    } else {
        None
    };
    match service
        .authorizer
        .authorize(&AuthorizationRequest {
            repository_id,
            operation,
            principal: principal.clone(),
        })
        .await
    {
        Ok(()) => {}
        Err(error) => return error_response(StatusCode::FORBIDDEN, &error.to_string()),
    }
    let principal_identity = principal.human_identity().cloned();
    let runtime_receive_context = if receive {
        match &principal {
            Principal::Human(_) => None,
            Principal::Runtime(runtime) => {
                let context = runtime.git_authority().map_or_else(
                    || runtime.receive_context().cloned(),
                    |authority| {
                        receive_policy::ResolvedRuntimeReceiveContext::new_with_expected_parent(
                            Arc::clone(&authority.scope),
                            authority.runtime_session_id.to_string(),
                            authority.authorization_snapshot_id.to_string(),
                            authority.evaluated_at.unix_timestamp(),
                            authority.expected_parent.clone(),
                        )
                        .ok()
                    },
                );
                let Some(context) = context else {
                    return error_response(
                        StatusCode::FORBIDDEN,
                        "runtime receive authority is unavailable",
                    );
                };
                let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
                    Ok(duration) => i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
                    Err(_) => {
                        return error_response(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "host clock is unavailable",
                        );
                    }
                };
                if context.repository_id()
                    != git_capability_domain::RepositoryId::new(repository_id.as_uuid())
                    || !context.is_active_at(now)
                {
                    return error_response(
                        StatusCode::FORBIDDEN,
                        "runtime receive authority is invalid",
                    );
                }
                if service.runtime_receive_hook.is_none() {
                    return error_response(
                        StatusCode::FORBIDDEN,
                        "runtime receive policy guard is unavailable",
                    );
                }
                Some(context)
            }
        }
    } else {
        None
    };
    let repository_result = match &principal_identity {
        Some(identity) => {
            service
                .repository
                .get_repository_as(repository_id, identity)
                .await
        }
        None => service.repository.get_repository(repository_id).await,
    };
    let repository = match repository_result {
        Ok(repository) => repository,
        Err(ForgeRepositoryError::RepositoryNotFound(_)) => {
            return error_response(StatusCode::NOT_FOUND, "repository was not found");
        }
        Err(error) => return service_error(error),
    };
    stream::execute_backend(stream::BackendExecutionInput {
        service,
        repository_id,
        operation,
        endpoint,
        query,
        principal,
        request,
        receive,
        principal_identity,
        runtime_receive_provenance,
        runtime_receive_context,
        repository,
    })
    .await
}
