//! Installed UI RPC conversion for installation, navigation, and handoff.
//!
//! The module is deliberately an application-port adapter. It does not query
//! `PostgreSQL` or perform tenant preflights; owner and generation authority is
//! rechecked by the release ports in their transactions.

use super::ReleaseRpc;
use crate::rpc::auth::{UiHandoffAuditMarker, append_ui_request_audit_bounded};
use crate::rpc::{RpcError, into_connect_error, mutation_receipt, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use forge_domain::OrganizationId;
use identity_domain::UserId;
use release_domain::{
    UiInstallationCallerKey, UiInstallationGenerationId, UiInstallationId, UiInstallationState,
    UiInstallationTarget,
    ui::{UiIcon, UiPresentation},
    ui_browser::UiBrowserHandoffSecret,
};
use release_service::{
    ActivateUiInstallation, CreateUiBrowserHandoff, DisableUiInstallation, InstallUi,
    ListUiInstallations, RemoveUiInstallation, RollbackUiInstallation, UiInstallationContentKind,
    UiInstallationNavigation, UiInstallationNavigationError, UiInstallationNavigator,
    UiInstallationPage, UiInstallationReceiptScope, UiInstallationTargetFilter,
    UiRequestAuditContext, UiRequestAuditDecision, UiRequestAuditOutcome, UiRequestAuditReason,
    UiRequestAuditSink, UiRequestAuditSurface,
};
use rpc_proto::messages::hephaestus::{
    common::v1::{OpaqueId, PageResponse},
    release::v1::{
        ActivateUiRequest, ActivateUiResponse, DisableUiRequest, DisableUiResponse,
        InstallUiRequest, InstallUiResponse, ListUiInstallationsRequest,
        ListUiInstallationsResponse, RemoveUiRequest, RemoveUiResponse, RollbackUiRequest,
        RollbackUiResponse, UiInstallationNavigation as ProtoNavigation,
        UiInstallationTarget as ProtoTarget,
    },
};

const INSTALL_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/InstallUi";
const LIST_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/ListUiInstallations";
const HANDOFF_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/CreateUiBrowserHandoff";
const PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;
const CURSOR_TAG_LENGTH: usize = 32;

mod conversions;
mod cursor;
mod handoff;
mod list;
mod mutations;

#[cfg(test)]
#[path = "ui/tests.rs"]
mod tests;

pub(super) use conversions::{
    expected_generation_id, generation_id, handoff_error, install_error, installation_id,
    lifecycle, lifecycle_receipt, mutation_context, navigation, navigation_error, opaque,
    organization_id, parse_handoff_secret, parse_target, release_id, timestamp, ui_key,
};
#[cfg(test)]
pub(super) use cursor::cursor_scope;
pub(super) use cursor::{TargetReceiptScope, UiInstallationCursorCodec};

/// Handles `ActivateUi` through the budgeted mutation implementation.
pub(super) async fn activate(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ActivateUiRequest>,
) -> ServiceResult<ActivateUiResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    request::run_with_budget(&budget, mutations::activate(service, ctx, request))
        .await
        .map_err(into_connect_error)?
}

/// Handles `RollbackUi` through the budgeted mutation implementation.
pub(super) async fn rollback(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, RollbackUiRequest>,
) -> ServiceResult<RollbackUiResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    request::run_with_budget(&budget, mutations::rollback(service, ctx, request))
        .await
        .map_err(into_connect_error)?
}

/// Handles `DisableUi` through the budgeted mutation implementation.
pub(super) async fn disable(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, DisableUiRequest>,
) -> ServiceResult<DisableUiResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    request::run_with_budget(&budget, mutations::disable(service, ctx, request))
        .await
        .map_err(into_connect_error)?
}

/// Handles `RemoveUi` through the budgeted mutation implementation.
pub(super) async fn remove(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, RemoveUiRequest>,
) -> ServiceResult<RemoveUiResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    request::run_with_budget(&budget, mutations::remove(service, ctx, request))
        .await
        .map_err(into_connect_error)?
}

/// Handles `InstallUi` through the budgeted mutation implementation.
pub(super) async fn install(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, InstallUiRequest>,
) -> ServiceResult<InstallUiResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    request::run_with_budget(&budget, mutations::install(service, ctx, request))
        .await
        .map_err(into_connect_error)?
}

/// Handles `ListUiInstallations` through the budgeted list implementation.
pub(super) async fn list(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ListUiInstallationsRequest>,
) -> ServiceResult<ListUiInstallationsResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    request::run_with_budget(&budget, list::list(service, ctx, request))
        .await
        .map_err(into_connect_error)?
}

/// Handles UI handoff issuance through the budgeted handoff implementation.
pub(super) async fn handoff(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<
        '_,
        rpc_proto::messages::hephaestus::release::v1::CreateUiBrowserHandoffRequest,
    >,
) -> ServiceResult<rpc_proto::messages::hephaestus::release::v1::CreateUiBrowserHandoffResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    request::run_with_budget(&budget, handoff::handoff(service, ctx, request))
        .await
        .map_err(into_connect_error)?
}

#[cfg(test)]
pub(super) use conversions::parse_uuid_value;
