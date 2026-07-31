///Shorthand for `OwnedView<ListProjectRunsRequestView<'static>>`.
pub type OwnedListProjectRunsRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::run::v1::__buffa::view::ListProjectRunsRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListProjectRunsResponseView<'static>>`.
pub type OwnedListProjectRunsResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::run::v1::__buffa::view::ListProjectRunsResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetRunRequestView<'static>>`.
pub type OwnedGetRunRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::run::v1::__buffa::view::GetRunRequestView<'static>,
>;
///Shorthand for `OwnedView<GetRunResponseView<'static>>`.
pub type OwnedGetRunResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::run::v1::__buffa::view::GetRunResponseView<'static>,
>;
///Shorthand for `OwnedView<RequestControlRequestView<'static>>`.
pub type OwnedRequestControlRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::run::v1::__buffa::view::RequestControlRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RequestControlResponseView<'static>>`.
pub type OwnedRequestControlResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::run::v1::__buffa::view::RequestControlResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::run::v1::ListProjectRunsResponse,
>
for crate::messages::hephaestus::run::v1::__buffa::view::ListProjectRunsResponseView<
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
    crate::messages::hephaestus::run::v1::ListProjectRunsResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::run::v1::__buffa::view::ListProjectRunsResponseView<
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
impl ::connectrpc::Encodable<crate::messages::hephaestus::run::v1::GetRunResponse>
for crate::messages::hephaestus::run::v1::__buffa::view::GetRunResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<crate::messages::hephaestus::run::v1::GetRunResponse>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::run::v1::__buffa::view::GetRunResponseView<'static>,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self.reborrow(), codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::run::v1::RequestControlResponse,
>
for crate::messages::hephaestus::run::v1::__buffa::view::RequestControlResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::run::v1::RequestControlResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::run::v1::__buffa::view::RequestControlResponseView<
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
pub const RUN_SERVICE_SERVICE_NAME: &str = "hephaestus.run.v1.RunService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListProjectRuns` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const RUN_SERVICE_LIST_PROJECT_RUNS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.run.v1.RunService/ListProjectRuns",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetRun` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const RUN_SERVICE_GET_RUN_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.run.v1.RunService/GetRun",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `RequestControl` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const RUN_SERVICE_REQUEST_CONTROL_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.run.v1.RunService/RequestControl",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Server trait for RunService.
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
pub trait RunService: Send + Sync + 'static {
    /// Handle the ListProjectRuns RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_project_runs<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::run::v1::ListProjectRunsRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::run::v1::ListProjectRunsResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the GetRun RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_run<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::run::v1::GetRunRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::run::v1::GetRunResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the RequestControl RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn request_control<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::run::v1::RequestControlRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::run::v1::RequestControlResponse,
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
pub trait RunServiceExt: RunService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: RunService> RunServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view_idempotent(
                RUN_SERVICE_SERVICE_NAME,
                "ListProjectRuns",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::run::v1::__buffa::view::ListProjectRunsRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::run::v1::ListProjectRunsRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_project_runs(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::run::v1::ListProjectRunsResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(RUN_SERVICE_LIST_PROJECT_RUNS_SPEC)
            .route_view_idempotent(
                RUN_SERVICE_SERVICE_NAME,
                "GetRun",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::run::v1::__buffa::view::GetRunRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::run::v1::GetRunRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_run(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::run::v1::GetRunResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(RUN_SERVICE_GET_RUN_SPEC)
            .route_view(
                RUN_SERVICE_SERVICE_NAME,
                "RequestControl",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::run::v1::__buffa::view::RequestControlRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::run::v1::RequestControlRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.request_control(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::run::v1::RequestControlResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(RUN_SERVICE_REQUEST_CONTROL_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct RunServiceRegisterMarker;
impl<S: RunService> ::connectrpc::ServiceRegister<RunServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as RunServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `RunService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = RunServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct RunServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: RunService> RunServiceServer<T> {
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
impl<T> Clone for RunServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: RunService> ::connectrpc::Dispatcher for RunServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("hephaestus.run.v1.RunService/")?;
        match method {
            "ListProjectRuns" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(RUN_SERVICE_LIST_PROJECT_RUNS_SPEC),
                )
            }
            "GetRun" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(RUN_SERVICE_GET_RUN_SPEC),
                )
            }
            "RequestControl" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(RUN_SERVICE_REQUEST_CONTROL_SPEC),
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
        let Some(method) = path.strip_prefix("hephaestus.run.v1.RunService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "ListProjectRuns" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::run::v1::ListProjectRunsRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::run::v1::__buffa::view::ListProjectRunsRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::run::v1::ListProjectRunsRequest,
                    >::from_parts(&req, &body);
                    svc.list_project_runs(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::run::v1::ListProjectRunsResponse,
                        >(format)
                })
            }
            "GetRun" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::run::v1::GetRunRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::run::v1::__buffa::view::GetRunRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::run::v1::GetRunRequest,
                    >::from_parts(&req, &body);
                    svc.get_run(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::run::v1::GetRunResponse,
                        >(format)
                })
            }
            "RequestControl" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::run::v1::RequestControlRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::run::v1::__buffa::view::RequestControlRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::run::v1::RequestControlRequest,
                    >::from_parts(&req, &body);
                    svc.request_control(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::run::v1::RequestControlResponse,
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
        let Some(method) = path.strip_prefix("hephaestus.run.v1.RunService/") else {
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
        let Some(method) = path.strip_prefix("hephaestus.run.v1.RunService/") else {
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
        let Some(method) = path.strip_prefix("hephaestus.run.v1.RunService/") else {
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
/// let client = RunServiceClient::new(conn, config);
/// let response = client.list_project_runs(request).await?;
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
/// let client = RunServiceClient::new(http, config);
/// let response = client.list_project_runs(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.list_project_runs(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.list_project_runs(request).await?.into_owned();
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
pub struct RunServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> RunServiceClient<T>
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
    /// Call the ListProjectRuns RPC. Sends a request to /hephaestus.run.v1.RunService/ListProjectRuns.
    pub async fn list_project_runs(
        &self,
        request: crate::messages::hephaestus::run::v1::ListProjectRunsRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::run::v1::__buffa::view::ListProjectRunsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_project_runs_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListProjectRuns RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_project_runs_with_options(
        &self,
        request: crate::messages::hephaestus::run::v1::ListProjectRunsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::run::v1::__buffa::view::ListProjectRunsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                RUN_SERVICE_SERVICE_NAME,
                "ListProjectRuns",
                request,
                options,
            )
            .await
    }
    /// Call the GetRun RPC. Sends a request to /hephaestus.run.v1.RunService/GetRun.
    pub async fn get_run(
        &self,
        request: crate::messages::hephaestus::run::v1::GetRunRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::run::v1::__buffa::view::GetRunResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_run_with_options(request, ::connectrpc::client::CallOptions::default())
            .await
    }
    /// Call the GetRun RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_run_with_options(
        &self,
        request: crate::messages::hephaestus::run::v1::GetRunRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::run::v1::__buffa::view::GetRunResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                RUN_SERVICE_SERVICE_NAME,
                "GetRun",
                request,
                options,
            )
            .await
    }
    /// Call the RequestControl RPC. Sends a request to /hephaestus.run.v1.RunService/RequestControl.
    pub async fn request_control(
        &self,
        request: crate::messages::hephaestus::run::v1::RequestControlRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::run::v1::__buffa::view::RequestControlResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.request_control_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the RequestControl RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn request_control_with_options(
        &self,
        request: crate::messages::hephaestus::run::v1::RequestControlRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::run::v1::__buffa::view::RequestControlResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                RUN_SERVICE_SERVICE_NAME,
                "RequestControl",
                request,
                options,
            )
            .await
    }
}
