///Shorthand for `OwnedView<ListBuilderImagesRequestView<'static>>`.
pub type OwnedListBuilderImagesRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::builder::v1::__buffa::view::ListBuilderImagesRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListBuilderImagesResponseView<'static>>`.
pub type OwnedListBuilderImagesResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::builder::v1::__buffa::view::ListBuilderImagesResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetBuilderImageRequestView<'static>>`.
pub type OwnedGetBuilderImageRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::builder::v1::__buffa::view::GetBuilderImageRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetBuilderImageResponseView<'static>>`.
pub type OwnedGetBuilderImageResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::builder::v1::__buffa::view::GetBuilderImageResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ValidateAgentConfigRequestView<'static>>`.
pub type OwnedValidateAgentConfigRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::builder::v1::__buffa::view::ValidateAgentConfigRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ValidateAgentConfigResponseView<'static>>`.
pub type OwnedValidateAgentConfigResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::builder::v1::__buffa::view::ValidateAgentConfigResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::builder::v1::ListBuilderImagesResponse,
>
for crate::messages::hephaestus::builder::v1::__buffa::view::ListBuilderImagesResponseView<
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
    crate::messages::hephaestus::builder::v1::ListBuilderImagesResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::builder::v1::__buffa::view::ListBuilderImagesResponseView<
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
    crate::messages::hephaestus::builder::v1::GetBuilderImageResponse,
>
for crate::messages::hephaestus::builder::v1::__buffa::view::GetBuilderImageResponseView<
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
    crate::messages::hephaestus::builder::v1::GetBuilderImageResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::builder::v1::__buffa::view::GetBuilderImageResponseView<
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
    crate::messages::hephaestus::builder::v1::ValidateAgentConfigResponse,
>
for crate::messages::hephaestus::builder::v1::__buffa::view::ValidateAgentConfigResponseView<
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
    crate::messages::hephaestus::builder::v1::ValidateAgentConfigResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::builder::v1::__buffa::view::ValidateAgentConfigResponseView<
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
pub const BUILDER_CATALOG_SERVICE_SERVICE_NAME: &str = "hephaestus.builder.v1.BuilderCatalogService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListBuilderImages` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const BUILDER_CATALOG_SERVICE_LIST_BUILDER_IMAGES_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.builder.v1.BuilderCatalogService/ListBuilderImages",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetBuilderImage` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const BUILDER_CATALOG_SERVICE_GET_BUILDER_IMAGE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.builder.v1.BuilderCatalogService/GetBuilderImage",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ValidateAgentConfig` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const BUILDER_CATALOG_SERVICE_VALIDATE_AGENT_CONFIG_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.builder.v1.BuilderCatalogService/ValidateAgentConfig",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Server trait for BuilderCatalogService.
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
pub trait BuilderCatalogService: Send + Sync + 'static {
    /// Handle the ListBuilderImages RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_builder_images<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::builder::v1::ListBuilderImagesRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::builder::v1::ListBuilderImagesResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the GetBuilderImage RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_builder_image<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::builder::v1::GetBuilderImageRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::builder::v1::GetBuilderImageResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ValidateAgentConfig RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn validate_agent_config<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::builder::v1::ValidateAgentConfigRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::builder::v1::ValidateAgentConfigResponse,
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
pub trait BuilderCatalogServiceExt: BuilderCatalogService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: BuilderCatalogService> BuilderCatalogServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view_idempotent(
                BUILDER_CATALOG_SERVICE_SERVICE_NAME,
                "ListBuilderImages",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::builder::v1::__buffa::view::ListBuilderImagesRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::builder::v1::ListBuilderImagesRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_builder_images(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::builder::v1::ListBuilderImagesResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(BUILDER_CATALOG_SERVICE_LIST_BUILDER_IMAGES_SPEC)
            .route_view_idempotent(
                BUILDER_CATALOG_SERVICE_SERVICE_NAME,
                "GetBuilderImage",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::builder::v1::__buffa::view::GetBuilderImageRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::builder::v1::GetBuilderImageRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_builder_image(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::builder::v1::GetBuilderImageResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(BUILDER_CATALOG_SERVICE_GET_BUILDER_IMAGE_SPEC)
            .route_view_idempotent(
                BUILDER_CATALOG_SERVICE_SERVICE_NAME,
                "ValidateAgentConfig",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::builder::v1::__buffa::view::ValidateAgentConfigRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::builder::v1::ValidateAgentConfigRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.validate_agent_config(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::builder::v1::ValidateAgentConfigResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(BUILDER_CATALOG_SERVICE_VALIDATE_AGENT_CONFIG_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct BuilderCatalogServiceRegisterMarker;
impl<
    S: BuilderCatalogService,
> ::connectrpc::ServiceRegister<BuilderCatalogServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as BuilderCatalogServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `BuilderCatalogService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = BuilderCatalogServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct BuilderCatalogServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: BuilderCatalogService> BuilderCatalogServiceServer<T> {
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
impl<T> Clone for BuilderCatalogServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: BuilderCatalogService> ::connectrpc::Dispatcher
for BuilderCatalogServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("hephaestus.builder.v1.BuilderCatalogService/")?;
        match method {
            "ListBuilderImages" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(BUILDER_CATALOG_SERVICE_LIST_BUILDER_IMAGES_SPEC),
                )
            }
            "GetBuilderImage" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(BUILDER_CATALOG_SERVICE_GET_BUILDER_IMAGE_SPEC),
                )
            }
            "ValidateAgentConfig" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(BUILDER_CATALOG_SERVICE_VALIDATE_AGENT_CONFIG_SPEC),
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
            .strip_prefix("hephaestus.builder.v1.BuilderCatalogService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "ListBuilderImages" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::builder::v1::ListBuilderImagesRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::builder::v1::__buffa::view::ListBuilderImagesRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::builder::v1::ListBuilderImagesRequest,
                    >::from_parts(&req, &body);
                    svc.list_builder_images(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::builder::v1::ListBuilderImagesResponse,
                        >(format)
                })
            }
            "GetBuilderImage" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::builder::v1::GetBuilderImageRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::builder::v1::__buffa::view::GetBuilderImageRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::builder::v1::GetBuilderImageRequest,
                    >::from_parts(&req, &body);
                    svc.get_builder_image(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::builder::v1::GetBuilderImageResponse,
                        >(format)
                })
            }
            "ValidateAgentConfig" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::builder::v1::ValidateAgentConfigRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::builder::v1::__buffa::view::ValidateAgentConfigRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::builder::v1::ValidateAgentConfigRequest,
                    >::from_parts(&req, &body);
                    svc.validate_agent_config(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::builder::v1::ValidateAgentConfigResponse,
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
            .strip_prefix("hephaestus.builder.v1.BuilderCatalogService/") else {
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
            .strip_prefix("hephaestus.builder.v1.BuilderCatalogService/") else {
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
            .strip_prefix("hephaestus.builder.v1.BuilderCatalogService/") else {
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
/// let client = BuilderCatalogServiceClient::new(conn, config);
/// let response = client.list_builder_images(request).await?;
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
/// let client = BuilderCatalogServiceClient::new(http, config);
/// let response = client.list_builder_images(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.list_builder_images(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.list_builder_images(request).await?.into_owned();
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
pub struct BuilderCatalogServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> BuilderCatalogServiceClient<T>
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
    /// Call the ListBuilderImages RPC. Sends a request to /hephaestus.builder.v1.BuilderCatalogService/ListBuilderImages.
    pub async fn list_builder_images(
        &self,
        request: crate::messages::hephaestus::builder::v1::ListBuilderImagesRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::builder::v1::__buffa::view::ListBuilderImagesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_builder_images_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListBuilderImages RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_builder_images_with_options(
        &self,
        request: crate::messages::hephaestus::builder::v1::ListBuilderImagesRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::builder::v1::__buffa::view::ListBuilderImagesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                BUILDER_CATALOG_SERVICE_SERVICE_NAME,
                "ListBuilderImages",
                request,
                options,
            )
            .await
    }
    /// Call the GetBuilderImage RPC. Sends a request to /hephaestus.builder.v1.BuilderCatalogService/GetBuilderImage.
    pub async fn get_builder_image(
        &self,
        request: crate::messages::hephaestus::builder::v1::GetBuilderImageRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::builder::v1::__buffa::view::GetBuilderImageResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_builder_image_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the GetBuilderImage RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_builder_image_with_options(
        &self,
        request: crate::messages::hephaestus::builder::v1::GetBuilderImageRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::builder::v1::__buffa::view::GetBuilderImageResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                BUILDER_CATALOG_SERVICE_SERVICE_NAME,
                "GetBuilderImage",
                request,
                options,
            )
            .await
    }
    /// Call the ValidateAgentConfig RPC. Sends a request to /hephaestus.builder.v1.BuilderCatalogService/ValidateAgentConfig.
    pub async fn validate_agent_config(
        &self,
        request: crate::messages::hephaestus::builder::v1::ValidateAgentConfigRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::builder::v1::__buffa::view::ValidateAgentConfigResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.validate_agent_config_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ValidateAgentConfig RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn validate_agent_config_with_options(
        &self,
        request: crate::messages::hephaestus::builder::v1::ValidateAgentConfigRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::builder::v1::__buffa::view::ValidateAgentConfigResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                BUILDER_CATALOG_SERVICE_SERVICE_NAME,
                "ValidateAgentConfig",
                request,
                options,
            )
            .await
    }
}
