use super::{RepositoryRpc, map_error, opaque, parse_id, parse_page, timestamp};
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::PageResponse,
    instance::v1::{RefSelector, TriggerPolicy, ref_selector},
    repository::v1::{
        ListRepositoryInstancesRequest, ListRepositoryInstancesResponse,
        RepositoryInstanceAttachment,
    },
};

pub(super) async fn handle(
    service: &RepositoryRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListRepositoryInstancesRequest>,
) -> ServiceResult<ListRepositoryInstancesResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.repository.v1.RepositoryService/ListRepositoryInstances",
    )
    .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let repository_id = parse_id(request.repository_id.as_option()).map_err(into_connect_error)?;
    let page = parse_page(request.page.as_option()).map_err(into_connect_error)?;
    let result = service
        .application
        .attachments(&identity, repository_id, page)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    let attachments = result
        .values
        .into_iter()
        .map(|row| {
            let selector_type = row
                .ref_selector
                .get("type")
                .and_then(serde_json::Value::as_str)
                .ok_or(RpcError::Internal)?;
            let selector_value = row
                .ref_selector
                .get("value")
                .and_then(serde_json::Value::as_str)
                .ok_or(RpcError::Internal)?
                .to_owned();
            let selector = match selector_type {
                "exact" => ref_selector::Selector::Exact(selector_value),
                "prefix" => ref_selector::Selector::Prefix(selector_value),
                _ => return Err(RpcError::Internal),
            };
            let trigger_policy = match row.trigger_policy.as_str() {
                "manual" => TriggerPolicy::Manual,
                "push" => TriggerPolicy::Push,
                "push_and_manual" => TriggerPolicy::PushAndManual,
                _ => return Err(RpcError::Internal),
            };
            Ok(RepositoryInstanceAttachment {
                id: opaque(row.id).into(),
                ref_selector: RefSelector {
                    selector: Some(selector),
                    ..Default::default()
                }
                .into(),
                trigger_policy: trigger_policy.into(),
                enabled: row.enabled,
                removed_at: row.removed_at.map(timestamp).into(),
                instance_id: opaque(row.instance_id).into(),
                instance_name: row.instance_name,
                instance_state: row.instance_state,
                project_id: opaque(row.project_id).into(),
                project_name: row.project_name,
                release_id: opaque(row.release_id).into(),
                release_version: row.release_version,
                ..Default::default()
            })
        })
        .collect::<Result<Vec<_>, RpcError>>()
        .map_err(into_connect_error)?;
    Response::ok(ListRepositoryInstancesResponse {
        attachments,
        page: PageResponse {
            next_page_token: result.next.unwrap_or_default(),
            stable_order: String::from("id"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}
