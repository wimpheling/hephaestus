//! Organization query RPC adapters.

use super::{MediatorAuthenticator, RpcError, into_connect_error, request};
use crate::application::organization::{
    OrganizationApplication, OrganizationError, OrganizationPage,
};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::{
    connect::hephaestus::organization::v1::OrganizationService,
    messages::hephaestus::{
        common::v1::{OpaqueId, PageRequest, PageResponse},
        organization::v1::{
            GetOrganizationRequest, GetOrganizationResponse, ListOrganizationProjectsRequest,
            ListOrganizationProjectsResponse, ListOrganizationRepositoriesRequest,
            ListOrganizationRepositoriesResponse, ListOrganizationsRequest,
            ListOrganizationsResponse, OrganizationSummary,
        },
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

const LIST_AUDIENCE: &str = "/hephaestus.organization.v1.OrganizationService/ListOrganizations";
const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;

/// Generated organization service backed by RLS-aware queries.
pub struct OrganizationRpc {
    application: OrganizationApplication,
    authenticator: MediatorAuthenticator,
}

impl OrganizationRpc {
    /// Creates an organization adapter and its query application.
    pub const fn new(pool: PgPool, authenticator: MediatorAuthenticator) -> Self {
        Self {
            application: OrganizationApplication::new(pool),
            authenticator,
        }
    }
}

// Generated traits hide an Encodable response; concrete message bodies keep
// each adapter readable while refining only this implementation's opaque type.
#[allow(refining_impl_trait)]
impl OrganizationService for OrganizationRpc {
    async fn list_organizations(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListOrganizationsRequest>,
    ) -> ServiceResult<ListOrganizationsResponse> {
        let identity = request::query_identity(&ctx, &self.authenticator, LIST_AUDIENCE)
            .map_err(into_connect_error)?;
        let request = request.to_owned_message();
        let page = parse_page(request.page.as_option()).map_err(into_connect_error)?;
        let result = self
            .application
            .list_organizations(&identity, page)
            .await
            .map_err(map_application_error)
            .map_err(into_connect_error)?;
        Response::ok(ListOrganizationsResponse {
            organizations: result
                .organizations
                .into_iter()
                .map(|organization| OrganizationSummary {
                    id: OpaqueId {
                        value: organization.id.to_string(),
                        ..Default::default()
                    }
                    .into(),
                    name: organization.name,
                    project_count: organization.project_count,
                    repository_count: organization.repository_count,
                    ..Default::default()
                })
                .collect(),
            page: PageResponse {
                next_page_token: result.next_page_token.unwrap_or_default(),
                stable_order: String::from("name,id"),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
    }

    async fn get_organization(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetOrganizationRequest>,
    ) -> ServiceResult<GetOrganizationResponse> {
        let identity = request::query_identity(
            &ctx,
            &self.authenticator,
            "/hephaestus.organization.v1.OrganizationService/GetOrganization",
        )
        .map_err(into_connect_error)?;
        let request = request.to_owned_message();
        let organization_id = parse_id(request.organization_id.as_option())?;
        let organization = self
            .application
            .get_organization(&identity, organization_id)
            .await
            .map_err(map_application_error)
            .map_err(into_connect_error)?;
        Response::ok(GetOrganizationResponse {
            organization: rpc_proto::messages::hephaestus::organization::v1::Organization {
                id: opaque(organization.id).into(),
                name: organization.name,
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
    }

    async fn list_organization_repositories(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListOrganizationRepositoriesRequest>,
    ) -> ServiceResult<ListOrganizationRepositoriesResponse> {
        let identity = request::query_identity(
            &ctx,
            &self.authenticator,
            "/hephaestus.organization.v1.OrganizationService/ListOrganizationRepositories",
        )
        .map_err(into_connect_error)?;
        let request = request.to_owned_message();
        let organization_id = parse_id(request.organization_id.as_option())?;
        let page = parse_page(request.page.as_option()).map_err(into_connect_error)?;
        let result = self
            .application
            .list_repositories(&identity, organization_id, page)
            .await
            .map_err(map_application_error)
            .map_err(into_connect_error)?;
        Response::ok(ListOrganizationRepositoriesResponse {
            repositories: result
                .repositories
                .into_iter()
                .map(|repository| {
                    rpc_proto::messages::hephaestus::organization::v1::RepositorySummary {
                        id: opaque(repository.id).into(),
                        name: repository.name,
                        default_branch: repository.default_branch,
                        is_public: repository.is_public,
                        project_name: repository.project_name,
                        run_count: repository.run_count,
                        last_run_at: repository.last_run_at.map(timestamp).into(),
                        ..Default::default()
                    }
                })
                .collect(),
            page: page_response(result.next_page_token, "project_name,name,id").into(),
            ..Default::default()
        })
    }

    async fn list_organization_projects(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListOrganizationProjectsRequest>,
    ) -> ServiceResult<ListOrganizationProjectsResponse> {
        let identity = request::query_identity(
            &ctx,
            &self.authenticator,
            "/hephaestus.organization.v1.OrganizationService/ListOrganizationProjects",
        )
        .map_err(into_connect_error)?;
        let request = request.to_owned_message();
        let organization_id = parse_id(request.organization_id.as_option())?;
        let page = parse_page(request.page.as_option()).map_err(into_connect_error)?;
        let result = self
            .application
            .list_projects(&identity, organization_id, page)
            .await
            .map_err(map_application_error)
            .map_err(into_connect_error)?;
        Response::ok(ListOrganizationProjectsResponse {
            projects: result
                .projects
                .into_iter()
                .map(
                    |project| rpc_proto::messages::hephaestus::organization::v1::ProjectSummary {
                        id: opaque(project.id).into(),
                        name: project.name,
                        repository_count: project.repository_count,
                        instance_count: project.instance_count,
                        run_count: project.run_count,
                        last_activity_at: project.last_activity_at.map(timestamp).into(),
                        ..Default::default()
                    },
                )
                .collect(),
            page: page_response(result.next_page_token, "name,id").into(),
            ..Default::default()
        })
    }
}

fn parse_page(page: Option<&PageRequest>) -> Result<OrganizationPage, RpcError> {
    let page_size = page.map_or(DEFAULT_PAGE_SIZE, |value| {
        if value.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            value.page_size
        }
    });
    if page_size > MAX_PAGE_SIZE {
        return Err(RpcError::InvalidArgument);
    }
    let after = page
        .filter(|value| !value.page_token.is_empty())
        .map(|value| Uuid::parse_str(&value.page_token))
        .transpose()
        .map_err(|_| RpcError::InvalidArgument)?;
    Ok(OrganizationPage {
        size: i64::from(page_size),
        after,
    })
}

fn parse_id(id: Option<&OpaqueId>) -> Result<Uuid, connectrpc::ConnectError> {
    let id = request::required_id(id).map_err(into_connect_error)?;
    Uuid::parse_str(&id).map_err(|_| into_connect_error(RpcError::InvalidArgument))
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

fn page_response(next_page_token: Option<String>, stable_order: &str) -> PageResponse {
    PageResponse {
        next_page_token: next_page_token.unwrap_or_default(),
        stable_order: stable_order.to_owned(),
        ..Default::default()
    }
}

fn map_application_error(error: OrganizationError) -> RpcError {
    match error {
        OrganizationError::InvalidPage => RpcError::InvalidArgument,
        OrganizationError::NotFound => RpcError::NotFound,
        OrganizationError::Persistence(source) => {
            tracing::error!(error = %source, "organization query failed");
            RpcError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, parse_page};
    use rpc_proto::messages::hephaestus::common::v1::PageRequest;
    use uuid::Uuid;

    #[test]
    fn pagination_is_bounded_and_cursor_is_typed() {
        assert_eq!(
            parse_page(None).expect("default page").size,
            i64::from(DEFAULT_PAGE_SIZE)
        );
        assert!(
            parse_page(Some(&PageRequest {
                page_size: MAX_PAGE_SIZE + 1,
                ..Default::default()
            }))
            .is_err()
        );
        assert!(
            parse_page(Some(&PageRequest {
                page_token: String::from("not-a-cursor"),
                ..Default::default()
            }))
            .is_err()
        );
        let cursor = Uuid::new_v4();
        assert_eq!(
            parse_page(Some(&PageRequest {
                page_token: cursor.to_string(),
                ..Default::default()
            }))
            .expect("valid cursor")
            .after,
            Some(cursor)
        );
    }
}
