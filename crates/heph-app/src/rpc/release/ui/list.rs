use super::{
    LIST_AUDIENCE, ListUiInstallations, ListUiInstallationsRequest, ListUiInstallationsResponse,
    MAX_PAGE_SIZE, PAGE_SIZE, PageResponse, ReleaseRpc, RequestContext, Response, RpcError,
    ServiceRequest, ServiceResult, TargetReceiptScope, UiInstallationNavigator, UiInstallationPage,
    into_connect_error, navigation, navigation_error, organization_id, parse_target, request,
};

/// Handles `ListUiInstallations` with an explicit organization and target.
pub async fn list(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ListUiInstallationsRequest>,
) -> ServiceResult<ListUiInstallationsResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = request.to_owned_message();
    let identity = request::query_identity(&ctx, &service.authenticator, LIST_AUDIENCE)
        .map_err(into_connect_error)?;
    let organization_id = organization_id(request.organization_id.as_option())?;
    let target = parse_target(request.target.as_option(), organization_id)?;
    let page = request
        .page
        .as_option()
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let page_size = if page.page_size == 0 {
        PAGE_SIZE
    } else {
        page.page_size
    };
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    // The navigator validates the UUID cursor against organization and target;
    // this transport codec additionally binds actor, tenant, target, method,
    // and stable order before the cursor reaches the application port.
    let after = if page.page_token.is_empty() {
        None
    } else {
        Some(
            service
                .ui_cursor_codec
                .decode(&page.page_token, identity.user_id, organization_id, target)
                .map_err(into_connect_error)?,
        )
    };
    let result = request::run_with_budget(
        &budget,
        service.ui_navigator.list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id,
                target: target.filter(),
                page: UiInstallationPage {
                    size: i64::from(page_size),
                    after,
                },
            },
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(navigation_error)
    .map_err(into_connect_error)?;
    let next_page_token = result.next.map(|id| {
        service
            .ui_cursor_codec
            .encode(id, identity.user_id, organization_id, target)
    });
    Response::ok(ListUiInstallationsResponse {
        installations: result.installations.iter().map(navigation).collect(),
        page: PageResponse {
            next_page_token: next_page_token.unwrap_or_default(),
            stable_order: String::from("created_at desc,id desc"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}
