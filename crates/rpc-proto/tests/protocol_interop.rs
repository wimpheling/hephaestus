//! End-to-end generated client/server interoperability across wire protocols.

use connectrpc::{
    Protocol, RequestContext, Response, Router, ServiceRequest, ServiceResult,
    client::{ClientConfig, Http2Connection, HttpClient},
};
use rpc_proto::{
    connect::hephaestus::identity::v1::{
        IdentityService, IdentityServiceClient, IdentityServiceExt,
    },
    messages::hephaestus::{
        common::v1::OpaqueId,
        identity::v1::{ResolveIdentityRequest, ResolveIdentityResponse},
    },
};
use std::sync::Arc;

struct FixtureIdentity;

#[allow(refining_impl_trait)]
impl IdentityService for FixtureIdentity {
    async fn resolve_identity(
        &self,
        _ctx: RequestContext,
        request: ServiceRequest<'_, ResolveIdentityRequest>,
    ) -> ServiceResult<ResolveIdentityResponse> {
        let request = request.to_owned_message();
        Response::ok(ResolveIdentityResponse {
            user_id: OpaqueId {
                value: String::from("00000000-0000-0000-0000-000000000001"),
                ..Default::default()
            }
            .into(),
            display_name: request.display_name,
            ..Default::default()
        })
    }
}

fn request() -> ResolveIdentityRequest {
    ResolveIdentityRequest {
        display_name: String::from("Protocol Fixture"),
        ..Default::default()
    }
}

#[tokio::test]
async fn generated_service_interoperates_over_connect_and_native_grpc() {
    let router = Arc::new(FixtureIdentity).register(Router::new());
    let reflector = connectrpc_reflection::Reflector::from_descriptor_pool(Arc::new(
        rpc_proto::descriptor_pool().expect("checked-in descriptor pool"),
    ))
    .expect("reflection index");
    let router = connectrpc_reflection::install(router, reflector);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind interoperability listener");
    let address = listener.local_addr().expect("listener address");
    let server = tokio::spawn(async move {
        axum::serve(listener, router.into_axum_router())
            .await
            .expect("serve generated fixture");
    });
    let uri: axum::http::Uri = format!("http://{address}").parse().expect("fixture URI");

    let connect = IdentityServiceClient::new(
        HttpClient::plaintext(),
        ClientConfig::new(uri.clone()).with_protocol(Protocol::Connect),
    );
    let connect_response = connect
        .resolve_identity(request())
        .await
        .expect("Connect response")
        .into_owned();
    assert_eq!(connect_response.display_name, "Protocol Fixture");

    let connection = Http2Connection::connect_plaintext(uri.clone())
        .await
        .expect("native gRPC HTTP/2 connection")
        .shared(16);
    let grpc = IdentityServiceClient::new(
        connection,
        ClientConfig::new(uri).with_protocol(Protocol::Grpc),
    );
    let grpc_response = grpc
        .resolve_identity(request())
        .await
        .expect("native gRPC response")
        .into_owned();
    assert_eq!(grpc_response.display_name, connect_response.display_name);
    assert_eq!(
        grpc_response
            .user_id
            .as_option()
            .map(|id| id.value.as_str()),
        Some("00000000-0000-0000-0000-000000000001")
    );

    server.abort();
    let _result = server.await;
}
