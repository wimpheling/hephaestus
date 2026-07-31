//! Project RPC composition and shared conversions.

mod get_project;
mod list_importable_release_agents;
mod list_project_instances;
mod list_project_repositories;

use super::{MediatorAuthenticator, RpcError};
use crate::application::project::{Page, ProjectApplication, ProjectError};
use connectrpc::{RequestContext, Router, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::{
    connect::hephaestus::project::v1::{ProjectService, ProjectServiceExt},
    messages::hephaestus::{
        common::v1::{OpaqueId, PageRequest},
        project::v1::{
            GetProjectRequest, GetProjectResponse, ListImportableReleaseAgentsRequest,
            ListImportableReleaseAgentsResponse, ListProjectInstancesRequest,
            ListProjectInstancesResponse, ListProjectRepositoriesRequest,
            ListProjectRepositoriesResponse,
        },
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 200;

pub struct ProjectRpc {
    application: ProjectApplication,
    authenticator: MediatorAuthenticator,
}

impl ProjectRpc {
    const fn new(pool: PgPool, authenticator: MediatorAuthenticator) -> Self {
        Self {
            application: ProjectApplication::new(pool),
            authenticator,
        }
    }
}

/// Registers the generated project service.
pub fn register(router: Router, pool: PgPool, authenticator: MediatorAuthenticator) -> Router {
    ProjectServiceExt::register(
        std::sync::Arc::new(ProjectRpc::new(pool, authenticator)),
        router,
    )
}

#[allow(refining_impl_trait)]
impl ProjectService for ProjectRpc {
    async fn get_project(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetProjectRequest>,
    ) -> ServiceResult<GetProjectResponse> {
        get_project::handle(self, ctx, request).await
    }

    async fn list_project_repositories(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListProjectRepositoriesRequest>,
    ) -> ServiceResult<ListProjectRepositoriesResponse> {
        list_project_repositories::handle(self, ctx, request).await
    }

    async fn list_project_instances(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListProjectInstancesRequest>,
    ) -> ServiceResult<ListProjectInstancesResponse> {
        list_project_instances::handle(self, ctx, request).await
    }

    async fn list_importable_release_agents(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListImportableReleaseAgentsRequest>,
    ) -> ServiceResult<ListImportableReleaseAgentsResponse> {
        list_importable_release_agents::handle(self, ctx, request).await
    }
}

fn parse_page(page: Option<&PageRequest>) -> Result<Page, RpcError> {
    let size = page.map_or(DEFAULT_PAGE_SIZE, |page| {
        if page.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            page.page_size
        }
    });
    if size > MAX_PAGE_SIZE {
        return Err(RpcError::InvalidArgument);
    }
    let after = page
        .filter(|page| !page.page_token.is_empty())
        .map(|page| Uuid::parse_str(&page.page_token))
        .transpose()
        .map_err(|_| RpcError::InvalidArgument)?;
    Ok(Page {
        size: i64::from(size),
        after,
    })
}

fn parse_id(id: Option<&OpaqueId>) -> Result<Uuid, RpcError> {
    id.ok_or(RpcError::InvalidArgument)?
        .value
        .parse()
        .map_err(|_| RpcError::InvalidArgument)
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
        nanos: value.nanosecond().cast_signed(),
        ..Default::default()
    }
}

fn map_error(error: ProjectError) -> RpcError {
    match error {
        ProjectError::PermissionDenied => RpcError::PermissionDenied,
        ProjectError::NotFound => RpcError::NotFound,
        ProjectError::InvalidPage => RpcError::InvalidArgument,
        ProjectError::Persistence(source) => {
            tracing::error!(error = %source, "project query failed");
            RpcError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_PAGE_SIZE, parse_page};
    use rpc_proto::messages::hephaestus::common::v1::PageRequest;

    #[test]
    fn pagination_rejects_oversized_and_malformed_cursors() {
        assert!(parse_page(None).is_ok());
        assert!(
            parse_page(Some(&PageRequest {
                page_size: MAX_PAGE_SIZE + 1,
                ..Default::default()
            }))
            .is_err()
        );
        assert!(
            parse_page(Some(&PageRequest {
                page_token: String::from("bad"),
                ..Default::default()
            }))
            .is_err()
        );
    }
}
