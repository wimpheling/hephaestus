pub(super) async fn handle(
    service: &super::ReleaseRpc,
    ctx: connectrpc::RequestContext,
    request: connectrpc::ServiceRequest<
        '_,
        rpc_proto::messages::hephaestus::release::v1::GetReleaseRequest,
    >,
) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::release::v1::GetReleaseResponse> {
    use rpc_proto::messages::hephaestus::release::v1::{GetReleaseResponse, Release};
    use uuid::Uuid;

    use super::super::{RpcError, into_connect_error, request as shared_request};

    const AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/GetRelease";

    let identity = shared_request::query_identity(&ctx, &service.authenticator, AUDIENCE)
        .map_err(into_connect_error)?;
    let request = request.to_owned_message();
    let id = shared_request::required_id(request.release_id.as_option())
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    let result = service
        .application
        .get_release(&identity, id)
        .await
        .map_err(super::model::application_error)
        .map_err(into_connect_error)?;
    let release_id = result.summary.id;
    let build_id = result.summary.build_request_id;
    let source_commit = result.summary.source_commit.clone();
    let release = Release {
        id: super::model::opaque(release_id).into(),
        version: result.summary.version,
        state: super::model::state(result.summary.state).into(),
        source_commit: result.summary.source_commit,
        source_ref: result.summary.source_ref,
        build_request_id: super::model::opaque(build_id).into(),
        build_definition_hash: result.build_definition_hash,
        configuration_hash: result.configuration_hash,
        manifest_hash: result.summary.manifest_hash,
        created_at: super::model::timestamp(result.summary.created_at).into(),
        published_at: result
            .summary
            .published_at
            .map(super::model::timestamp)
            .into(),
        revoked_at: result.revoked_at.map(super::model::timestamp).into(),
        repository_id: super::model::opaque(result.repository_id).into(),
        repository_name: result.repository_name,
        project_id: super::model::opaque(result.project_id).into(),
        project_name: result.project_name,
        organization_id: super::model::opaque(result.organization_id).into(),
        organization_name: result.organization_name,
        build: super::model::build(result.build).into(),
        artifacts: result
            .artifacts
            .into_iter()
            .map(|artifact| super::model::artifact(artifact, release_id, build_id, &source_commit))
            .collect(),
        agents: result.agents.into_iter().map(super::model::agent).collect(),
        ..Default::default()
    };
    connectrpc::Response::ok(GetReleaseResponse {
        release: release.into(),
        ..Default::default()
    })
}
