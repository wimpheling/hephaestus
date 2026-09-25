use super::errors::invalid;
use crate::application::run::Page;
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::RequestContext;
use rpc_proto::messages::hephaestus::common::v1::{OpaqueId, PageRequest};
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 200;

pub(super) fn query(
    ctx: &RequestContext,
    auth: &super::MediatorAuthenticator,
    method: &str,
) -> Result<identity_domain::AuthenticatedIdentity, connectrpc::ConnectError> {
    request::query_identity(
        ctx,
        auth,
        &format!("/hephaestus.run.v1.RunService/{method}"),
    )
    .map_err(into_connect_error)
}

pub(super) fn parse_id(value: Option<&OpaqueId>) -> Result<Uuid, connectrpc::ConnectError> {
    request::required_id(value)
        .map_err(into_connect_error)?
        .parse()
        .map_err(invalid)
}

pub(super) fn parse_page(value: Option<&PageRequest>) -> Result<Page, connectrpc::ConnectError> {
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
