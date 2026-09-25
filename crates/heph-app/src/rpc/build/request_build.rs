pub(super) async fn handle(
    service: &super::BuildRpc,
    ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<
        '_,
        rpc_proto::messages::hephaestus::build::v1::RequestBuildRequest,
    >,
) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::build::v1::RequestBuildResponse> {
    use crate::application::build::{RequestBuild, decode_hash};
    use rpc_proto::messages::hephaestus::{build::v1::RequestBuildResponse, common::v1::Operation};
    use uuid::Uuid;

    use super::super::{RpcError, into_connect_error, request as shared_request};

    const AUDIENCE: &str = "/hephaestus.build.v1.BuildService/RequestBuild";

    let request = request.to_owned_message();
    let identity = shared_request::mutation_identity(
        &ctx,
        &service.authenticator,
        AUDIENCE,
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let repository_id = shared_request::required_id(request.repository_id.as_option())
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    if !valid_commit(&request.source_commit) {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let build_definition_hash = decode_hash(&request.build_definition_hash)
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let configuration_hash = decode_hash(&request.configuration_hash)
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let result = service
        .application
        .request_build(
            &identity,
            RequestBuild {
                repository_id,
                source_commit: request.source_commit,
                build_definition_hash,
                configuration_hash,
            },
        )
        .await
        .map_err(super::model::application_error)
        .map_err(into_connect_error)?;
    let receipt = crate::rpc::mutation_receipt(
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "build",
        "repository",
    )
    .await?;
    connectrpc::Response::ok(RequestBuildResponse {
        build_id: super::model::opaque(result.id).into(),
        operation: Operation {
            id: super::model::opaque(result.id).into(),
            state: super::model::operation_state(result.state).into(),
            created_at: super::model::timestamp(result.created_at).into(),
            updated_at: super::model::timestamp(result.updated_at).into(),
            ..Default::default()
        }
        .into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

fn valid_commit(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod request_build_tests {
    use super::valid_commit;

    #[test]
    fn commit_requires_canonical_git_object_hex() {
        assert!(valid_commit(&"ab".repeat(20)));
        assert!(valid_commit(&"ab".repeat(32)));
        assert!(!valid_commit(&"AB".repeat(20)));
        assert!(!valid_commit("deadbeef"));
    }
}
