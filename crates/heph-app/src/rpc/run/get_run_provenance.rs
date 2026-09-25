//! Exact, redacted run authorization and HTTPS inspection.

use super::{RunRpc, map_error, opaque, parse_id, parse_page, query, timestamp};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::PageResponse,
    run::v1::{GetRunProvenanceRequest, GetRunProvenanceResponse, RunHttpsUse},
};

pub(super) async fn handle(
    rpc: &RunRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetRunProvenanceRequest>,
) -> ServiceResult<GetRunProvenanceResponse> {
    let identity = query(&ctx, &rpc.authenticator, "GetRunProvenance")?;
    let request = message.to_owned_message();
    let run_id = parse_id(request.run_id.as_option())?;
    let value = rpc
        .application
        .get_run_provenance(&identity, run_id, parse_page(request.page.as_option())?)
        .await
        .map_err(map_error)?;
    Response::ok(GetRunProvenanceResponse {
        run_id: opaque(run_id).into(),
        authorization_snapshot_id: value.snapshot.as_ref().map(|s| opaque(s.id)).into(),
        authorization_model_version: value
            .snapshot
            .as_ref()
            .map(|s| s.authorization_model_version.clone())
            .unwrap_or_default(),
        authorization_snapshot_hash: value
            .snapshot
            .map(|s| s.normalized_hash)
            .unwrap_or_default(),
        https_uses: value
            .uses
            .values
            .into_iter()
            .map(|row| RunHttpsUse {
                id: opaque(row.id).into(),
                request_id: opaque(row.request_id).into(),
                lease_id: opaque(row.lease_id).into(),
                binding_id: opaque(row.binding_id).into(),
                secret_version_id: opaque(row.secret_version_id).into(),
                rule_id: opaque(row.rule_id).into(),
                event_kind: row.event_kind,
                decision: row.decision.unwrap_or_default(),
                outcome: row.outcome.unwrap_or_default(),
                occurred_at: timestamp(row.occurred_at).into(),
                ..Default::default()
            })
            .collect(),
        page: PageResponse {
            next_page_token: value.uses.next.unwrap_or_default(),
            stable_order: "id_asc".into(),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}
