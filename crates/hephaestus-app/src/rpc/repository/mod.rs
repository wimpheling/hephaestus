//! Repository RPC composition and shared conversions.

mod get_repository;
mod list_repository_instances;

use super::{MediatorAuthenticator, RpcError};
use crate::application::repository::{Page, RepositoryApplication, RepositoryError};
use connectrpc::{RequestContext, Router, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::{
    connect::hephaestus::repository::v1::{RepositoryService, RepositoryServiceExt},
    messages::hephaestus::{
        common::v1::{OpaqueId, PageRequest},
        repository::v1::{
            GetRepositoryRequest, GetRepositoryResponse, ListRepositoryInstancesRequest,
            ListRepositoryInstancesResponse,
        },
    },
};
use time::OffsetDateTime;
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 200;

pub struct RepositoryRpc {
    application: RepositoryApplication,
    authenticator: MediatorAuthenticator,
}

impl RepositoryRpc {
    const fn new(pool: PgPool, authenticator: MediatorAuthenticator) -> Self {
        Self {
            application: RepositoryApplication::new(pool),
            authenticator,
        }
    }
}

pub fn register(router: Router, pool: PgPool, authenticator: MediatorAuthenticator) -> Router {
    RepositoryServiceExt::register(
        std::sync::Arc::new(RepositoryRpc::new(pool, authenticator)),
        router,
    )
}

#[allow(refining_impl_trait)]
impl RepositoryService for RepositoryRpc {
    async fn get_repository(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetRepositoryRequest>,
    ) -> ServiceResult<GetRepositoryResponse> {
        get_repository::handle(self, ctx, request).await
    }

    async fn list_repository_instances(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListRepositoryInstancesRequest>,
    ) -> ServiceResult<ListRepositoryInstancesResponse> {
        list_repository_instances::handle(self, ctx, request).await
    }
}

fn parse_id(value: Option<&OpaqueId>) -> Result<Uuid, RpcError> {
    value
        .ok_or(RpcError::InvalidArgument)?
        .value
        .parse()
        .map_err(|_| RpcError::InvalidArgument)
}

fn parse_page(value: Option<&PageRequest>) -> Result<Page, RpcError> {
    let size = value.map_or(DEFAULT_PAGE_SIZE, |page| {
        if page.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            page.page_size
        }
    });
    if size > MAX_PAGE_SIZE {
        return Err(RpcError::InvalidArgument);
    }
    let after = value
        .filter(|page| !page.page_token.is_empty())
        .map(|page| page.page_token.parse())
        .transpose()
        .map_err(|_| RpcError::InvalidArgument)?;
    Ok(Page {
        size: i64::from(size),
        after,
    })
}

fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
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

fn map_error(error: RepositoryError) -> RpcError {
    match error {
        RepositoryError::PermissionDenied => RpcError::PermissionDenied,
        RepositoryError::NotFound => RpcError::NotFound,
        RepositoryError::InvalidPage => RpcError::InvalidArgument,
        RepositoryError::Persistence(source) => {
            tracing::error!(error = %source, "repository query failed");
            RpcError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_PAGE_SIZE, parse_page, timestamp};
    use rpc_proto::messages::hephaestus::common::v1::PageRequest;

    #[test]
    fn page_is_bounded_and_cursor_is_a_uuid() {
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

    #[test]
    fn repository_timestamp_is_constructed_without_json_round_trip() {
        let value = time::OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("valid fixture timestamp")
            + time::Duration::nanoseconds(987_654_321);
        let projected = timestamp(value);
        assert_eq!(projected.seconds, value.unix_timestamp());
        assert_eq!(projected.nanos, 987_654_321);
    }
}
