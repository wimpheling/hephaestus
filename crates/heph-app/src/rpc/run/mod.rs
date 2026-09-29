//! Run query and durable control RPC adapters.

mod auth;
mod conversions;
mod errors;
mod get_run_provenance;
mod mutations;
mod queries;

use super::{MediatorAuthenticator, MutationReceipts};
use crate::application::run::RunApplication;
use connectrpc::{RequestContext, Router, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::{
    connect::hephaestus::run::v1::{RunService, RunServiceExt},
    messages::hephaestus::run::v1::{
        GetRunProvenanceRequest, GetRunProvenanceResponse, GetRunRequest, GetRunResponse,
        ListProjectRunsRequest, ListProjectRunsResponse, RequestControlRequest,
        RequestControlResponse,
    },
};
use std::path::PathBuf;

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
    async fn get_run_provenance(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, GetRunProvenanceRequest>,
    ) -> ServiceResult<GetRunProvenanceResponse> {
        get_run_provenance::handle(self, ctx, message).await
    }

    async fn list_project_runs(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, ListProjectRunsRequest>,
    ) -> ServiceResult<ListProjectRunsResponse> {
        queries::list_project_runs(self, ctx, message).await
    }

    async fn get_run(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, GetRunRequest>,
    ) -> ServiceResult<GetRunResponse> {
        queries::get_run(self, ctx, message).await
    }

    async fn request_control(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, RequestControlRequest>,
    ) -> ServiceResult<RequestControlResponse> {
        mutations::request_control(self, ctx, message).await
    }
}
