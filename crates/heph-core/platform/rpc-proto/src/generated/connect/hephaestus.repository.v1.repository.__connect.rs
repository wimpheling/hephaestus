///Shorthand for `OwnedView<CreateRepositoryRequestView<'static>>`.
pub type OwnedCreateRepositoryRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository::v1::__buffa::view::CreateRepositoryRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreateRepositoryResponseView<'static>>`.
pub type OwnedCreateRepositoryResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository::v1::__buffa::view::CreateRepositoryResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetRepositoryRequestView<'static>>`.
pub type OwnedGetRepositoryRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository::v1::__buffa::view::GetRepositoryRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetRepositoryResponseView<'static>>`.
pub type OwnedGetRepositoryResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository::v1::__buffa::view::GetRepositoryResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListRepositoryInstancesRequestView<'static>>`.
pub type OwnedListRepositoryInstancesRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository::v1::__buffa::view::ListRepositoryInstancesRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListRepositoryInstancesResponseView<'static>>`.
pub type OwnedListRepositoryInstancesResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository::v1::__buffa::view::ListRepositoryInstancesResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::repository::v1::CreateRepositoryResponse,
>
for crate::messages::hephaestus::repository::v1::__buffa::view::CreateRepositoryResponseView<
    '_,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::repository::v1::CreateRepositoryResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository::v1::__buffa::view::CreateRepositoryResponseView<
        'static,
    >,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self.reborrow(), codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::repository::v1::GetRepositoryResponse,
>
for crate::messages::hephaestus::repository::v1::__buffa::view::GetRepositoryResponseView<
    '_,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::repository::v1::GetRepositoryResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository::v1::__buffa::view::GetRepositoryResponseView<
        'static,
    >,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self.reborrow(), codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::repository::v1::ListRepositoryInstancesResponse,
>
for crate::messages::hephaestus::repository::v1::__buffa::view::ListRepositoryInstancesResponseView<
    '_,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::repository::v1::ListRepositoryInstancesResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository::v1::__buffa::view::ListRepositoryInstancesResponseView<
        'static,
    >,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self.reborrow(), codec)
    }
}
/// Full service name for this service.
pub const REPOSITORY_SERVICE_SERVICE_NAME: &str = "hephaestus.repository.v1.RepositoryService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `CreateRepository` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const REPOSITORY_SERVICE_CREATE_REPOSITORY_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.repository.v1.RepositoryService/CreateRepository",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetRepository` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const REPOSITORY_SERVICE_GET_REPOSITORY_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.repository.v1.RepositoryService/GetRepository",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListRepositoryInstances` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const REPOSITORY_SERVICE_LIST_REPOSITORY_INSTANCES_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.repository.v1.RepositoryService/ListRepositoryInstances",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Server trait for RepositoryService.
///
/// # Implementing handlers
///
/// Implement methods with plain `async fn`; the returned future satisfies
/// the `Send` bound automatically.
///
/// **Unary and server-streaming requests** arrive as
/// [`ServiceRequest<'_, Req>`](::connectrpc::ServiceRequest): a zero-copy
/// view of the request plus its body, valid for the duration of the call.
/// Fields are read directly (`request.name` is a `&str` into the decoded
/// buffer) and the borrow may be held across `.await` points. Anything
/// that must outlive the call — `tokio::spawn`, channels, server state,
/// or data captured by a returned response stream — takes owned data:
/// call `request.to_owned_message()` (or copy the specific fields)
/// first.
///
/// **Client-streaming and bidi requests** arrive as
/// [`InboundStream<Req>`](::connectrpc::InboundStream) — a
/// `ServiceStream` of [`StreamMessage`](::connectrpc::StreamMessage)s.
/// Each item owns its decoded buffer and is `Send + 'static`, so items
/// can be buffered or moved into spawned tasks; read fields zero-copy
/// through the generated accessor methods (`item.name()`) or `.view()`,
/// convert with `.to_owned_message()`, or yield an item back unchanged —
/// `StreamMessage<M>` implements `Encodable<M>`.
///
/// Request types resolved through `extern_path` (e.g. well-known types
/// from another crate) use the same wrappers; the crate that owns the
/// type must be generated with buffa ≥ 0.8.0 and views enabled so the
/// backing `HasMessageView` impl exists.
///
/// The `impl Encodable<Out>` return bound accepts the owned `Out`, the
/// generated `OutView<'_>` / `OwnedOutView`,
/// [`MaybeBorrowed`](::connectrpc::MaybeBorrowed), or
/// [`PreEncoded`](::connectrpc::PreEncoded) for handlers that encode a
/// non-`'static` view internally and pass the bytes across the handler
/// boundary. View bodies are not emitted for output types mapped via
/// `extern_path` (the impl would be an orphan); return owned for
/// WKT/extern outputs.
///
/// Server-streaming and bidi-streaming methods return
/// `ServiceStream<impl Encodable<Out> + Send + use<Self>>`. The
/// `use<Self>` precise-capturing clause excludes `&self`'s lifetime and
/// the request's lifetime (unary methods use `use<'a, Self>` and may
/// borrow from `&self`), so stream items must be `'static` and cannot
/// borrow from the request. To stream view-encoded data, encode each
/// item inside the stream body and yield
/// [`PreEncoded`](::connectrpc::PreEncoded) — see its `# Streaming
/// example` doc.
#[allow(clippy::type_complexity)]
pub trait RepositoryService: Send + Sync + 'static {
    /// Handle the CreateRepository RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn create_repository<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::repository::v1::CreateRepositoryRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::repository::v1::CreateRepositoryResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the GetRepository RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_repository<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::repository::v1::GetRepositoryRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::repository::v1::GetRepositoryResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ListRepositoryInstances RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_repository_instances<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::repository::v1::ListRepositoryInstancesRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::repository::v1::ListRepositoryInstancesResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
}
/// Extension trait for registering a service implementation with a Router.
///
/// This trait is automatically implemented for all types that implement the service trait.
/// Prefer [`Router::add_service`](::connectrpc::Router::add_service) for
/// top-down registration; `register` remains available for compatibility
/// and cases where the service-first call shape is more convenient.
///
/// # Example
///
/// ```rust,ignore
/// use std::sync::Arc;
///
/// let service = Arc::new(MyServiceImpl);
/// let router = service.register(Router::new());
/// ```
pub trait RepositoryServiceExt: RepositoryService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: RepositoryService> RepositoryServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view(
                REPOSITORY_SERVICE_SERVICE_NAME,
                "CreateRepository",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::repository::v1::__buffa::view::CreateRepositoryRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::repository::v1::CreateRepositoryRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.create_repository(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::repository::v1::CreateRepositoryResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(REPOSITORY_SERVICE_CREATE_REPOSITORY_SPEC)
            .route_view_idempotent(
                REPOSITORY_SERVICE_SERVICE_NAME,
                "GetRepository",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::repository::v1::__buffa::view::GetRepositoryRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::repository::v1::GetRepositoryRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_repository(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::repository::v1::GetRepositoryResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(REPOSITORY_SERVICE_GET_REPOSITORY_SPEC)
            .route_view_idempotent(
                REPOSITORY_SERVICE_SERVICE_NAME,
                "ListRepositoryInstances",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::repository::v1::__buffa::view::ListRepositoryInstancesRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::repository::v1::ListRepositoryInstancesRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_repository_instances(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::repository::v1::ListRepositoryInstancesResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(REPOSITORY_SERVICE_LIST_REPOSITORY_INSTANCES_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct RepositoryServiceRegisterMarker;
impl<S: RepositoryService> ::connectrpc::ServiceRegister<RepositoryServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as RepositoryServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `RepositoryService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = RepositoryServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct RepositoryServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: RepositoryService> RepositoryServiceServer<T> {
    /// Wrap a service implementation in a monomorphic dispatcher.
    pub fn new(service: T) -> Self {
        Self {
            inner: ::std::sync::Arc::new(service),
        }
    }
    /// Wrap an already-`Arc`'d service implementation.
    pub fn from_arc(inner: ::std::sync::Arc<T>) -> Self {
        Self { inner }
    }
}
impl<T> Clone for RepositoryServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: RepositoryService> ::connectrpc::Dispatcher for RepositoryServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("hephaestus.repository.v1.RepositoryService/")?;
        match method {
            "CreateRepository" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(REPOSITORY_SERVICE_CREATE_REPOSITORY_SPEC),
                )
            }
            "GetRepository" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(REPOSITORY_SERVICE_GET_REPOSITORY_SPEC),
                )
            }
            "ListRepositoryInstances" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(REPOSITORY_SERVICE_LIST_REPOSITORY_INSTANCES_SPEC),
                )
            }
            _ => None,
        }
    }
    fn call_unary(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::Payload,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::UnaryResult {
        let Some(method) = path
            .strip_prefix("hephaestus.repository.v1.RepositoryService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "CreateRepository" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::repository::v1::CreateRepositoryRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::repository::v1::__buffa::view::CreateRepositoryRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::repository::v1::CreateRepositoryRequest,
                    >::from_parts(&req, &body);
                    svc.create_repository(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::repository::v1::CreateRepositoryResponse,
                        >(format)
                })
            }
            "GetRepository" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::repository::v1::GetRepositoryRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::repository::v1::__buffa::view::GetRepositoryRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::repository::v1::GetRepositoryRequest,
                    >::from_parts(&req, &body);
                    svc.get_repository(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::repository::v1::GetRepositoryResponse,
                        >(format)
                })
            }
            "ListRepositoryInstances" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::repository::v1::ListRepositoryInstancesRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::repository::v1::__buffa::view::ListRepositoryInstancesRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::repository::v1::ListRepositoryInstancesRequest,
                    >::from_parts(&req, &body);
                    svc.list_repository_instances(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::repository::v1::ListRepositoryInstancesResponse,
                        >(format)
                })
            }
            _ => ::connectrpc::dispatcher::codegen::unimplemented_unary(path),
        }
    }
    fn call_server_streaming(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        request: ::buffa::bytes::Bytes,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::StreamingResult {
        let Some(method) = path
            .strip_prefix("hephaestus.repository.v1.RepositoryService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_streaming(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            _ => ::connectrpc::dispatcher::codegen::unimplemented_streaming(path),
        }
    }
    fn call_client_streaming(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        requests: ::connectrpc::dispatcher::codegen::RequestStream,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::UnaryResult {
        let Some(method) = path
            .strip_prefix("hephaestus.repository.v1.RepositoryService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &requests, &format);
        match method {
            _ => ::connectrpc::dispatcher::codegen::unimplemented_unary(path),
        }
    }
    fn call_bidi_streaming(
        &self,
        path: &str,
        ctx: ::connectrpc::RequestContext,
        requests: ::connectrpc::dispatcher::codegen::RequestStream,
        format: ::connectrpc::CodecFormat,
    ) -> ::connectrpc::dispatcher::codegen::StreamingResult {
        let Some(method) = path
            .strip_prefix("hephaestus.repository.v1.RepositoryService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_streaming(path);
        };
        let _ = (&ctx, &requests, &format);
        match method {
            _ => ::connectrpc::dispatcher::codegen::unimplemented_streaming(path),
        }
    }
}
/// Client for this service.
///
/// Generic over `T: ClientTransport`. For **gRPC** (HTTP/2), use
/// `Http2Connection` — it has honest `poll_ready` and composes with
/// `tower::balance` for multi-connection load balancing. For **Connect
/// over HTTP/1.1** (or unknown protocol), use `HttpClient`.
///
/// # Example (gRPC / HTTP/2)
///
/// ```rust,ignore
/// use connectrpc::client::{Http2Connection, ClientConfig};
/// use connectrpc::Protocol;
///
/// let uri: http::Uri = "http://localhost:8080".parse()?;
/// let conn = Http2Connection::connect_plaintext(uri.clone()).await?.shared(1024);
/// let config = ClientConfig::new(uri).with_protocol(Protocol::Grpc);
///
/// let client = RepositoryServiceClient::new(conn, config);
/// let response = client.create_repository(request).await?;
/// ```
///
/// # Example (Connect / HTTP/1.1 or ALPN)
///
/// ```rust,ignore
/// use connectrpc::client::{HttpClient, ClientConfig};
///
/// let http = HttpClient::plaintext();  // cleartext http:// only
/// let config = ClientConfig::new("http://localhost:8080".parse()?);
///
/// let client = RepositoryServiceClient::new(http, config);
/// let response = client.create_repository(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.create_repository(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.create_repository(request).await?.into_owned();
/// ```
///
/// [`into_view()`](::connectrpc::client::UnaryResponse::into_view) keeps the
/// zero-copy decoded body (an `OwnedView`) without copying; field access on it
/// goes through `.reborrow()`. Streaming responses yield one
/// [`StreamMessage`](::connectrpc::StreamMessage) per received message from
/// `.message().await` — read fields zero-copy through the generated accessor
/// methods (`msg.name()`) or `.view()`, or convert with `.to_owned_message()`.
#[cfg(feature = "client")]
#[derive(Clone)]
pub struct RepositoryServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> RepositoryServiceClient<T>
where
    T: ::connectrpc::client::ClientTransport,
    <T::ResponseBody as ::connectrpc::http_body::Body>::Error: ::std::fmt::Display,
{
    /// Create a new client with the given transport and configuration.
    pub fn new(transport: T, config: ::connectrpc::client::ClientConfig) -> Self {
        Self { transport, config }
    }
    /// Get the client configuration.
    pub fn config(&self) -> &::connectrpc::client::ClientConfig {
        &self.config
    }
    /// Get a mutable reference to the client configuration.
    pub fn config_mut(&mut self) -> &mut ::connectrpc::client::ClientConfig {
        &mut self.config
    }
    /// Call the CreateRepository RPC. Sends a request to /hephaestus.repository.v1.RepositoryService/CreateRepository.
    pub async fn create_repository(
        &self,
        request: crate::messages::hephaestus::repository::v1::CreateRepositoryRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository::v1::__buffa::view::CreateRepositoryResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.create_repository_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the CreateRepository RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn create_repository_with_options(
        &self,
        request: crate::messages::hephaestus::repository::v1::CreateRepositoryRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository::v1::__buffa::view::CreateRepositoryResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                REPOSITORY_SERVICE_SERVICE_NAME,
                "CreateRepository",
                request,
                options,
            )
            .await
    }
    /// Call the GetRepository RPC. Sends a request to /hephaestus.repository.v1.RepositoryService/GetRepository.
    pub async fn get_repository(
        &self,
        request: crate::messages::hephaestus::repository::v1::GetRepositoryRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository::v1::__buffa::view::GetRepositoryResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_repository_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the GetRepository RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_repository_with_options(
        &self,
        request: crate::messages::hephaestus::repository::v1::GetRepositoryRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository::v1::__buffa::view::GetRepositoryResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                REPOSITORY_SERVICE_SERVICE_NAME,
                "GetRepository",
                request,
                options,
            )
            .await
    }
    /// Call the ListRepositoryInstances RPC. Sends a request to /hephaestus.repository.v1.RepositoryService/ListRepositoryInstances.
    pub async fn list_repository_instances(
        &self,
        request: crate::messages::hephaestus::repository::v1::ListRepositoryInstancesRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository::v1::__buffa::view::ListRepositoryInstancesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_repository_instances_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListRepositoryInstances RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_repository_instances_with_options(
        &self,
        request: crate::messages::hephaestus::repository::v1::ListRepositoryInstancesRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository::v1::__buffa::view::ListRepositoryInstancesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                REPOSITORY_SERVICE_SERVICE_NAME,
                "ListRepositoryInstances",
                request,
                options,
            )
            .await
    }
}
