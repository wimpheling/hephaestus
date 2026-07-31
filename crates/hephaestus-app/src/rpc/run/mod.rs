//! Run query and durable control RPC adapters.

use super::{
    MediatorAuthenticator, MutationReceipts, RpcError, into_connect_error, mutation_receipt,
    request,
};
use crate::application::run::{
    ControlKind as AppKind, ControlTarget as AppTarget, EventPayload, Page,
    RequestControl as AppControl, RunApplication, RunError, RunEvent as AppEvent, RunView,
};
use connectrpc::{RequestContext, Response, Router, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::{
    connect::hephaestus::run::v1::{RunService, RunServiceExt},
    messages::hephaestus::{
        artifact::v1::{Artifact, ArtifactProvenance},
        common::v1::{MetricLabel, OpaqueId, PageRequest, PageResponse, RuntimeMetric},
        run::v1::{
            ControlState, GetRunRequest, GetRunResponse, ListProjectRunsRequest,
            ListProjectRunsResponse, RequestControlRequest, RequestControlResponse, ResultProposal,
            Run, RunControlKind, RunEvent, RunFailure, RunMetrics, RunResult, RunSummary,
            run_control_target, run_event,
        },
    },
};
use std::path::PathBuf;
use time::OffsetDateTime;
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 200;

pub struct RunRpc {
    application: RunApplication,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl RunRpc {
    const fn new(
        pool: PgPool,
        result_artifact_root: PathBuf,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
    ) -> Self {
        Self {
            application: RunApplication::new(pool, result_artifact_root),
            authenticator,
            receipts,
        }
    }
}

pub fn register(
    router: Router,
    pool: PgPool,
    result_artifact_root: PathBuf,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
) -> Router {
    RunServiceExt::register(
        std::sync::Arc::new(RunRpc::new(
            pool,
            result_artifact_root,
            authenticator,
            receipts,
        )),
        router,
    )
}

#[allow(refining_impl_trait)]
impl RunService for RunRpc {
    async fn list_project_runs(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, ListProjectRunsRequest>,
    ) -> ServiceResult<ListProjectRunsResponse> {
        let identity = query(&ctx, &self.authenticator, "ListProjectRuns")?;
        let request = message.to_owned_message();
        let result = self
            .application
            .list_project_runs(
                &identity,
                parse_id(request.project_id.as_option())?,
                parse_page(request.page.as_option())?,
            )
            .await
            .map_err(map_error)?;
        Response::ok(ListProjectRunsResponse {
            page: PageResponse {
                next_page_token: result.next.unwrap_or_default(),
                stable_order: String::from("created_at_desc,id_desc"),
                ..Default::default()
            }
            .into(),
            runs: result
                .values
                .into_iter()
                .map(|row| RunSummary {
                    id: opaque(row.id).into(),
                    state: row.state,
                    outcome: row.outcome.unwrap_or_default(),
                    run_kind: row.run_kind,
                    updated_at: timestamp(row.updated_at).into(),
                    instance_id: opaque(row.instance_id).into(),
                    instance_name: row.instance_name,
                    repository_id: row.repository_id.map(opaque).into(),
                    repository_name: row.repository_name.unwrap_or_default(),
                    commit_sha: row.commit_sha.unwrap_or_default(),
                    git_ref: row.git_ref.unwrap_or_default(),
                    release_id: opaque(row.release_id).into(),
                    release_version: row.release_version,
                    instance_revision_id: opaque(row.instance_revision_id).into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        })
    }

    async fn get_run(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, GetRunRequest>,
    ) -> ServiceResult<GetRunResponse> {
        let identity = query(&ctx, &self.authenticator, "GetRun")?;
        let request = message.to_owned_message();
        let run = self
            .application
            .get_run(&identity, parse_id(request.run_id.as_option())?)
            .await
            .map_err(map_error)?;
        Response::ok(GetRunResponse {
            run: proto_run(run)?.into(),
            ..Default::default()
        })
    }

    async fn request_control(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, RequestControlRequest>,
    ) -> ServiceResult<RequestControlResponse> {
        let request = message.to_owned_message();
        let identity = request::mutation_identity(
            &ctx,
            &self.authenticator,
            "/hephaestus.run.v1.RunService/RequestControl",
            request.context.as_option(),
        )
        .map_err(into_connect_error)?;
        if request.reason.len() > 4096 {
            return Err(into_connect_error(RpcError::InvalidArgument));
        }
        let kind = match request.kind.as_known() {
            Some(RunControlKind::Cancel) => AppKind::Cancel,
            Some(RunControlKind::Retry) => AppKind::Retry,
            Some(RunControlKind::ApproveResult) => AppKind::Approve,
            Some(RunControlKind::RejectResult) => AppKind::Reject,
            _ => return Err(into_connect_error(RpcError::InvalidArgument)),
        };
        let target = match request
            .target
            .as_option()
            .and_then(|target| target.target.as_ref())
        {
            Some(run_control_target::Target::RunId(id)) => {
                AppTarget::Run(id.value.parse().map_err(invalid)?)
            }
            Some(run_control_target::Target::ProposalId(id)) => {
                AppTarget::Proposal(id.value.parse().map_err(invalid)?)
            }
            None => return Err(into_connect_error(RpcError::InvalidArgument)),
        };
        if matches!(kind, AppKind::Cancel | AppKind::Retry) != matches!(target, AppTarget::Run(_)) {
            return Err(into_connect_error(RpcError::InvalidArgument));
        }
        let receipt_aggregate = if matches!(target, AppTarget::Run(_)) {
            "run"
        } else {
            "review"
        };
        let result = self
            .application
            .request_control(
                &identity,
                AppControl {
                    kind,
                    repository_id: parse_id(request.repository_id.as_option())?,
                    target,
                    reason: request.reason,
                },
            )
            .await
            .map_err(map_error)?;
        let state = match result.state.as_str() {
            "pending" | "processing" => ControlState::Queued,
            "completed" => ControlState::Applied,
            "failed" => ControlState::Rejected,
            _ => return Err(into_connect_error(RpcError::Internal)),
        };
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            receipt_aggregate,
            "run",
        )
        .await?;
        Response::ok(RequestControlResponse {
            control_request_id: opaque(result.id).into(),
            state: state.into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }
}

// The protobuf snapshot intentionally maps the complete run aggregate together.
#[allow(clippy::too_many_lines)]
fn proto_run(value: RunView) -> Result<Run, connectrpc::ConnectError> {
    let event_count = u64::try_from(value.events.len()).map_err(invalid)?;
    let log_count = u64::try_from(
        value
            .events
            .iter()
            .filter(|event| matches!(event.payload, EventPayload::Log(_)))
            .count(),
    )
    .map_err(invalid)?;
    let runtime_metrics = value
        .events
        .iter()
        .filter_map(|event| {
            if let EventPayload::Metric {
                name,
                value,
                labels,
            } = &event.payload
            {
                Some(metric(name.clone(), *value, labels.clone()))
            } else {
                None
            }
        })
        .collect();
    let events = value
        .events
        .into_iter()
        .map(proto_event)
        .collect::<Result<Vec<_>, _>>()?;
    let artifacts = value
        .artifacts
        .into_iter()
        .map(|artifact| Artifact {
            id: opaque(artifact.id).into(),
            path: artifact.path,
            kind: artifact.kind,
            mode: artifact
                .mode
                .and_then(|mode| u32::try_from(mode).ok())
                .unwrap_or_default(),
            sha256: artifact.sha256,
            size_bytes: u64::try_from(artifact.size_bytes).unwrap_or_default(),
            media_type: artifact.media_type.unwrap_or_default(),
            provenance: ArtifactProvenance {
                run_id: opaque(value.id).into(),
                release_id: opaque(value.release_id).into(),
                source_commit: value.input_commit.clone(),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
        .collect();
    let result = value.result_id.map(|id| RunResult {
        id: opaque(id).into(),
        commit: value.result_commit.unwrap_or_default(),
        r#ref: value.result_ref.unwrap_or_default(),
        tree: value.result_tree.unwrap_or_default(),
        message: value.result_message.unwrap_or_default(),
        artifact_manifest_hash: value.artifact_manifest_hash.unwrap_or_default(),
        proposal: value
            .proposal_id
            .map(|id| ResultProposal {
                id: opaque(id).into(),
                state: value.proposal_state.unwrap_or_default(),
                target_ref: value.proposal_target_ref.unwrap_or_default(),
                version: value
                    .proposal_version
                    .and_then(|version| u64::try_from(version).ok())
                    .unwrap_or_default(),
                ..Default::default()
            })
            .into(),
        ..Default::default()
    });
    let elapsed = (value.updated_at - value.created_at)
        .whole_milliseconds()
        .max(0);
    Ok(Run {
        id: opaque(value.id).into(),
        state: value.state,
        outcome: value.outcome.unwrap_or_default(),
        exit_code: value.exit_code,
        exit_signal: value.exit_signal,
        failure: value
            .failure
            .map(|code| RunFailure {
                code,
                ..Default::default()
            })
            .into(),
        created_at: timestamp(value.created_at).into(),
        updated_at: timestamp(value.updated_at).into(),
        state_version: u64::try_from(value.state_version).unwrap_or_default(),
        agent_id: opaque(value.agent_id).into(),
        agent_name: value.agent_name,
        instance_project_id: opaque(value.instance_project_id).into(),
        instance_project_name: value.instance_project_name,
        instance_revision_id: opaque(value.instance_revision_id).into(),
        release_id: opaque(value.release_id).into(),
        release_version: value.release_version,
        source_repository_id: opaque(value.source_repository_id).into(),
        repository_id: opaque(value.repository_id).into(),
        repository_name: value.repository_name,
        project_id: opaque(value.project_id).into(),
        project_name: value.project_name,
        organization_id: opaque(value.organization_id).into(),
        organization_name: value.organization_name,
        input_commit: value.input_commit,
        git_ref: value.git_ref,
        attempt: u32::try_from(value.attempt).unwrap_or_default(),
        result: result.into(),
        events,
        artifacts,
        metrics: RunMetrics {
            event_count,
            log_count,
            elapsed_ms: u64::try_from(elapsed).unwrap_or_default(),
            runtime_metrics,
            ..Default::default()
        }
        .into(),
        patch_preview: value.patch_preview,
        manifest_preview: value.manifest_preview,
        ..Default::default()
    })
}

fn metric(
    name: String,
    value: f64,
    labels: std::collections::BTreeMap<String, String>,
) -> RuntimeMetric {
    RuntimeMetric {
        name,
        value,
        labels: labels
            .into_iter()
            .map(|(key, value)| MetricLabel {
                key,
                value,
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}
fn proto_event(event: AppEvent) -> Result<RunEvent, connectrpc::ConnectError> {
    let payload = match event.payload {
        EventPayload::Log(message) => run_event::Payload::BoundedLogMessage(message),
        EventPayload::State(state) => run_event::Payload::State(state),
        EventPayload::Metric {
            name,
            value,
            labels,
        } => run_event::Payload::Metric(Box::new(metric(name, value, labels))),
    };
    Ok(RunEvent {
        sequence: u64::try_from(event.sequence).map_err(invalid)?,
        event_type: event.event_type,
        payload: Some(payload),
        occurred_at: timestamp(event.occurred_at).into(),
        ..Default::default()
    })
}
fn query(
    ctx: &RequestContext,
    auth: &MediatorAuthenticator,
    method: &str,
) -> Result<identity_domain::AuthenticatedIdentity, connectrpc::ConnectError> {
    request::query_identity(
        ctx,
        auth,
        &format!("/hephaestus.run.v1.RunService/{method}"),
    )
    .map_err(into_connect_error)
}
fn parse_id(value: Option<&OpaqueId>) -> Result<Uuid, connectrpc::ConnectError> {
    request::required_id(value)
        .map_err(into_connect_error)?
        .parse()
        .map_err(invalid)
}
fn parse_page(value: Option<&PageRequest>) -> Result<Page, connectrpc::ConnectError> {
    let size = value.map_or(DEFAULT_PAGE_SIZE, |page| {
        if page.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            page.page_size
        }
    });
    if size > MAX_PAGE_SIZE {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let after = value
        .filter(|page| !page.page_token.is_empty())
        .map(|page| page.page_token.parse())
        .transpose()
        .map_err(invalid)?;
    Ok(Page {
        size: i64::from(size),
        after,
    })
}
fn opaque(id: Uuid) -> OpaqueId {
    OpaqueId {
        value: id.to_string(),
        ..Default::default()
    }
}
fn timestamp(value: OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: i32::try_from(value.nanosecond()).unwrap_or_default(),
        ..Default::default()
    }
}
fn map_error(error: RunError) -> connectrpc::ConnectError {
    match error {
        RunError::NotFound => into_connect_error(RpcError::NotFound),
        RunError::InvalidPage => into_connect_error(RpcError::InvalidArgument),
        RunError::IdempotencyConflict => into_connect_error(RpcError::FailedPrecondition),
        RunError::PreviewUnavailable => into_connect_error(RpcError::Unavailable),
        RunError::Persistence(source) => {
            tracing::error!(error=%source,"run operation failed");
            into_connect_error(RpcError::Unavailable)
        }
    }
}
fn invalid<T>(_error: T) -> connectrpc::ConnectError {
    into_connect_error(RpcError::InvalidArgument)
}
