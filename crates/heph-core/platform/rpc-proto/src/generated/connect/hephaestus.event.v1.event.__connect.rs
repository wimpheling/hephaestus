///Shorthand for `OwnedView<WatchIdentityRequestView<'static>>`.
pub type OwnedWatchIdentityRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchIdentityRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchIdentityResponseView<'static>>`.
pub type OwnedWatchIdentityResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchIdentityResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchOrganizationRequestView<'static>>`.
pub type OwnedWatchOrganizationRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchOrganizationRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchOrganizationResponseView<'static>>`.
pub type OwnedWatchOrganizationResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchOrganizationResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchProjectRequestView<'static>>`.
pub type OwnedWatchProjectRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchProjectRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchProjectResponseView<'static>>`.
pub type OwnedWatchProjectResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchProjectResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchRepositoryRequestView<'static>>`.
pub type OwnedWatchRepositoryRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchRepositoryRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchRepositoryResponseView<'static>>`.
pub type OwnedWatchRepositoryResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchRepositoryResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchRunRequestView<'static>>`.
pub type OwnedWatchRunRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchRunRequestView<'static>,
>;
///Shorthand for `OwnedView<WatchRunResponseView<'static>>`.
pub type OwnedWatchRunResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchRunResponseView<'static>,
>;
///Shorthand for `OwnedView<WatchAgentInstanceRequestView<'static>>`.
pub type OwnedWatchAgentInstanceRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchAgentInstanceRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchAgentInstanceResponseView<'static>>`.
pub type OwnedWatchAgentInstanceResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchAgentInstanceResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::event::v1::WatchIdentityResponse,
>
for crate::messages::hephaestus::event::v1::__buffa::view::WatchIdentityResponseView<
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
    crate::messages::hephaestus::event::v1::WatchIdentityResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchIdentityResponseView<
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
    crate::messages::hephaestus::event::v1::WatchOrganizationResponse,
>
for crate::messages::hephaestus::event::v1::__buffa::view::WatchOrganizationResponseView<
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
    crate::messages::hephaestus::event::v1::WatchOrganizationResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchOrganizationResponseView<
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
    crate::messages::hephaestus::event::v1::WatchProjectResponse,
>
for crate::messages::hephaestus::event::v1::__buffa::view::WatchProjectResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::event::v1::WatchProjectResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchProjectResponseView<
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
    crate::messages::hephaestus::event::v1::WatchRepositoryResponse,
>
for crate::messages::hephaestus::event::v1::__buffa::view::WatchRepositoryResponseView<
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
    crate::messages::hephaestus::event::v1::WatchRepositoryResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchRepositoryResponseView<
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
impl ::connectrpc::Encodable<crate::messages::hephaestus::event::v1::WatchRunResponse>
for crate::messages::hephaestus::event::v1::__buffa::view::WatchRunResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<crate::messages::hephaestus::event::v1::WatchRunResponse>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchRunResponseView<'static>,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self.reborrow(), codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::event::v1::WatchAgentInstanceResponse,
>
for crate::messages::hephaestus::event::v1::__buffa::view::WatchAgentInstanceResponseView<
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
    crate::messages::hephaestus::event::v1::WatchAgentInstanceResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::event::v1::__buffa::view::WatchAgentInstanceResponseView<
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
pub const PRODUCT_EVENT_SERVICE_SERVICE_NAME: &str = "hephaestus.event.v1.ProductEventService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `WatchIdentity` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PRODUCT_EVENT_SERVICE_WATCH_IDENTITY_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.event.v1.ProductEventService/WatchIdentity",
        ::connectrpc::StreamType::ServerStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `WatchOrganization` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PRODUCT_EVENT_SERVICE_WATCH_ORGANIZATION_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.event.v1.ProductEventService/WatchOrganization",
        ::connectrpc::StreamType::ServerStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `WatchProject` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PRODUCT_EVENT_SERVICE_WATCH_PROJECT_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.event.v1.ProductEventService/WatchProject",
        ::connectrpc::StreamType::ServerStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `WatchRepository` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PRODUCT_EVENT_SERVICE_WATCH_REPOSITORY_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.event.v1.ProductEventService/WatchRepository",
        ::connectrpc::StreamType::ServerStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `WatchRun` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PRODUCT_EVENT_SERVICE_WATCH_RUN_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.event.v1.ProductEventService/WatchRun",
        ::connectrpc::StreamType::ServerStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `WatchAgentInstance` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PRODUCT_EVENT_SERVICE_WATCH_AGENT_INSTANCE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.event.v1.ProductEventService/WatchAgentInstance",
        ::connectrpc::StreamType::ServerStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Exposes only explicitly authorized product-event scopes. There is no global
/// watch method.
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
pub trait ProductEventService: Send + Sync + 'static {
    /// Handle the WatchIdentity RPC.
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call (until the response stream is returned);
    /// message fields are read directly on it (zero-copy). Data the
    /// returned stream needs must be copied out or converted via
    /// `.to_owned_message()`.
    fn watch_identity(
        &self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::event::v1::WatchIdentityRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::messages::hephaestus::event::v1::WatchIdentityResponse,
                > + Send + use<Self>,
            >,
        >,
    > + Send;
    /// Handle the WatchOrganization RPC.
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call (until the response stream is returned);
    /// message fields are read directly on it (zero-copy). Data the
    /// returned stream needs must be copied out or converted via
    /// `.to_owned_message()`.
    fn watch_organization(
        &self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::event::v1::WatchOrganizationRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::messages::hephaestus::event::v1::WatchOrganizationResponse,
                > + Send + use<Self>,
            >,
        >,
    > + Send;
    /// Handle the WatchProject RPC.
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call (until the response stream is returned);
    /// message fields are read directly on it (zero-copy). Data the
    /// returned stream needs must be copied out or converted via
    /// `.to_owned_message()`.
    fn watch_project(
        &self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::event::v1::WatchProjectRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::messages::hephaestus::event::v1::WatchProjectResponse,
                > + Send + use<Self>,
            >,
        >,
    > + Send;
    /// Handle the WatchRepository RPC.
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call (until the response stream is returned);
    /// message fields are read directly on it (zero-copy). Data the
    /// returned stream needs must be copied out or converted via
    /// `.to_owned_message()`.
    fn watch_repository(
        &self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::event::v1::WatchRepositoryRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::messages::hephaestus::event::v1::WatchRepositoryResponse,
                > + Send + use<Self>,
            >,
        >,
    > + Send;
    /// Handle the WatchRun RPC.
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call (until the response stream is returned);
    /// message fields are read directly on it (zero-copy). Data the
    /// returned stream needs must be copied out or converted via
    /// `.to_owned_message()`.
    fn watch_run(
        &self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::event::v1::WatchRunRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::messages::hephaestus::event::v1::WatchRunResponse,
                > + Send + use<Self>,
            >,
        >,
    > + Send;
    /// Handle the WatchAgentInstance RPC.
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call (until the response stream is returned);
    /// message fields are read directly on it (zero-copy). Data the
    /// returned stream needs must be copied out or converted via
    /// `.to_owned_message()`.
    fn watch_agent_instance(
        &self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::event::v1::WatchAgentInstanceRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::messages::hephaestus::event::v1::WatchAgentInstanceResponse,
                > + Send + use<Self>,
            >,
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
pub trait ProductEventServiceExt: ProductEventService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: ProductEventService> ProductEventServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view_server_stream::<
                _,
                _,
                crate::messages::hephaestus::event::v1::WatchIdentityResponse,
            >(
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchIdentity",
                ::connectrpc::view_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::event::v1::__buffa::view::WatchIdentityRequestView<
                                'static,
                            >,
                        >|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::event::v1::WatchIdentityRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.watch_identity(ctx, sreq).await
                        }
                    }
                }),
            )
            .with_spec(PRODUCT_EVENT_SERVICE_WATCH_IDENTITY_SPEC)
            .route_view_server_stream::<
                _,
                _,
                crate::messages::hephaestus::event::v1::WatchOrganizationResponse,
            >(
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchOrganization",
                ::connectrpc::view_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::event::v1::__buffa::view::WatchOrganizationRequestView<
                                'static,
                            >,
                        >|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::event::v1::WatchOrganizationRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.watch_organization(ctx, sreq).await
                        }
                    }
                }),
            )
            .with_spec(PRODUCT_EVENT_SERVICE_WATCH_ORGANIZATION_SPEC)
            .route_view_server_stream::<
                _,
                _,
                crate::messages::hephaestus::event::v1::WatchProjectResponse,
            >(
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchProject",
                ::connectrpc::view_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::event::v1::__buffa::view::WatchProjectRequestView<
                                'static,
                            >,
                        >|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::event::v1::WatchProjectRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.watch_project(ctx, sreq).await
                        }
                    }
                }),
            )
            .with_spec(PRODUCT_EVENT_SERVICE_WATCH_PROJECT_SPEC)
            .route_view_server_stream::<
                _,
                _,
                crate::messages::hephaestus::event::v1::WatchRepositoryResponse,
            >(
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchRepository",
                ::connectrpc::view_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::event::v1::__buffa::view::WatchRepositoryRequestView<
                                'static,
                            >,
                        >|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::event::v1::WatchRepositoryRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.watch_repository(ctx, sreq).await
                        }
                    }
                }),
            )
            .with_spec(PRODUCT_EVENT_SERVICE_WATCH_REPOSITORY_SPEC)
            .route_view_server_stream::<
                _,
                _,
                crate::messages::hephaestus::event::v1::WatchRunResponse,
            >(
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchRun",
                ::connectrpc::view_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::event::v1::__buffa::view::WatchRunRequestView<
                                'static,
                            >,
                        >|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::event::v1::WatchRunRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.watch_run(ctx, sreq).await
                        }
                    }
                }),
            )
            .with_spec(PRODUCT_EVENT_SERVICE_WATCH_RUN_SPEC)
            .route_view_server_stream::<
                _,
                _,
                crate::messages::hephaestus::event::v1::WatchAgentInstanceResponse,
            >(
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchAgentInstance",
                ::connectrpc::view_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::event::v1::__buffa::view::WatchAgentInstanceRequestView<
                                'static,
                            >,
                        >|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::event::v1::WatchAgentInstanceRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.watch_agent_instance(ctx, sreq).await
                        }
                    }
                }),
            )
            .with_spec(PRODUCT_EVENT_SERVICE_WATCH_AGENT_INSTANCE_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct ProductEventServiceRegisterMarker;
impl<
    S: ProductEventService,
> ::connectrpc::ServiceRegister<ProductEventServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as ProductEventServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `ProductEventService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = ProductEventServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct ProductEventServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: ProductEventService> ProductEventServiceServer<T> {
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
impl<T> Clone for ProductEventServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: ProductEventService> ::connectrpc::Dispatcher for ProductEventServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("hephaestus.event.v1.ProductEventService/")?;
        match method {
            "WatchIdentity" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::server_streaming()
                        .with_spec(PRODUCT_EVENT_SERVICE_WATCH_IDENTITY_SPEC),
                )
            }
            "WatchOrganization" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::server_streaming()
                        .with_spec(PRODUCT_EVENT_SERVICE_WATCH_ORGANIZATION_SPEC),
                )
            }
            "WatchProject" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::server_streaming()
                        .with_spec(PRODUCT_EVENT_SERVICE_WATCH_PROJECT_SPEC),
                )
            }
            "WatchRepository" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::server_streaming()
                        .with_spec(PRODUCT_EVENT_SERVICE_WATCH_REPOSITORY_SPEC),
                )
            }
            "WatchRun" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::server_streaming()
                        .with_spec(PRODUCT_EVENT_SERVICE_WATCH_RUN_SPEC),
                )
            }
            "WatchAgentInstance" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::server_streaming()
                        .with_spec(PRODUCT_EVENT_SERVICE_WATCH_AGENT_INSTANCE_SPEC),
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
        let Some(method) = path.strip_prefix("hephaestus.event.v1.ProductEventService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
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
        let Some(method) = path.strip_prefix("hephaestus.event.v1.ProductEventService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_streaming(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "WatchIdentity" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::event::v1::WatchIdentityRequest,
                    >(request, format)?;
                    let req: crate::messages::hephaestus::event::v1::__buffa::view::WatchIdentityRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::event::v1::WatchIdentityRequest,
                    >::from_parts(&req, &body);
                    let resp = svc.watch_identity(ctx, req).await?;
                    Ok(
                        resp
                            .map_body(|s| ::connectrpc::dispatcher::codegen::encode_response_stream::<
                                crate::messages::hephaestus::event::v1::WatchIdentityResponse,
                                _,
                                _,
                            >(s, format)),
                    )
                })
            }
            "WatchOrganization" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::event::v1::WatchOrganizationRequest,
                    >(request, format)?;
                    let req: crate::messages::hephaestus::event::v1::__buffa::view::WatchOrganizationRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::event::v1::WatchOrganizationRequest,
                    >::from_parts(&req, &body);
                    let resp = svc.watch_organization(ctx, req).await?;
                    Ok(
                        resp
                            .map_body(|s| ::connectrpc::dispatcher::codegen::encode_response_stream::<
                                crate::messages::hephaestus::event::v1::WatchOrganizationResponse,
                                _,
                                _,
                            >(s, format)),
                    )
                })
            }
            "WatchProject" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::event::v1::WatchProjectRequest,
                    >(request, format)?;
                    let req: crate::messages::hephaestus::event::v1::__buffa::view::WatchProjectRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::event::v1::WatchProjectRequest,
                    >::from_parts(&req, &body);
                    let resp = svc.watch_project(ctx, req).await?;
                    Ok(
                        resp
                            .map_body(|s| ::connectrpc::dispatcher::codegen::encode_response_stream::<
                                crate::messages::hephaestus::event::v1::WatchProjectResponse,
                                _,
                                _,
                            >(s, format)),
                    )
                })
            }
            "WatchRepository" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::event::v1::WatchRepositoryRequest,
                    >(request, format)?;
                    let req: crate::messages::hephaestus::event::v1::__buffa::view::WatchRepositoryRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::event::v1::WatchRepositoryRequest,
                    >::from_parts(&req, &body);
                    let resp = svc.watch_repository(ctx, req).await?;
                    Ok(
                        resp
                            .map_body(|s| ::connectrpc::dispatcher::codegen::encode_response_stream::<
                                crate::messages::hephaestus::event::v1::WatchRepositoryResponse,
                                _,
                                _,
                            >(s, format)),
                    )
                })
            }
            "WatchRun" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::event::v1::WatchRunRequest,
                    >(request, format)?;
                    let req: crate::messages::hephaestus::event::v1::__buffa::view::WatchRunRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::event::v1::WatchRunRequest,
                    >::from_parts(&req, &body);
                    let resp = svc.watch_run(ctx, req).await?;
                    Ok(
                        resp
                            .map_body(|s| ::connectrpc::dispatcher::codegen::encode_response_stream::<
                                crate::messages::hephaestus::event::v1::WatchRunResponse,
                                _,
                                _,
                            >(s, format)),
                    )
                })
            }
            "WatchAgentInstance" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::event::v1::WatchAgentInstanceRequest,
                    >(request, format)?;
                    let req: crate::messages::hephaestus::event::v1::__buffa::view::WatchAgentInstanceRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::event::v1::WatchAgentInstanceRequest,
                    >::from_parts(&req, &body);
                    let resp = svc.watch_agent_instance(ctx, req).await?;
                    Ok(
                        resp
                            .map_body(|s| ::connectrpc::dispatcher::codegen::encode_response_stream::<
                                crate::messages::hephaestus::event::v1::WatchAgentInstanceResponse,
                                _,
                                _,
                            >(s, format)),
                    )
                })
            }
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
        let Some(method) = path.strip_prefix("hephaestus.event.v1.ProductEventService/")
        else {
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
        let Some(method) = path.strip_prefix("hephaestus.event.v1.ProductEventService/")
        else {
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
/// let client = ProductEventServiceClient::new(conn, config);
/// let response = client.watch_identity(request).await?;
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
/// let client = ProductEventServiceClient::new(http, config);
/// let response = client.watch_identity(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.watch_identity(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.watch_identity(request).await?.into_owned();
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
pub struct ProductEventServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> ProductEventServiceClient<T>
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
    /// Call the WatchIdentity RPC. Sends a request to /hephaestus.event.v1.ProductEventService/WatchIdentity.
    pub async fn watch_identity(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchIdentityRequest,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchIdentityResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.watch_identity_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the WatchIdentity RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn watch_identity_with_options(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchIdentityRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchIdentityResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_server_stream(
                &self.transport,
                &self.config,
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchIdentity",
                request,
                options,
            )
            .await
    }
    /// Call the WatchOrganization RPC. Sends a request to /hephaestus.event.v1.ProductEventService/WatchOrganization.
    pub async fn watch_organization(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchOrganizationRequest,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchOrganizationResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.watch_organization_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the WatchOrganization RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn watch_organization_with_options(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchOrganizationRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchOrganizationResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_server_stream(
                &self.transport,
                &self.config,
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchOrganization",
                request,
                options,
            )
            .await
    }
    /// Call the WatchProject RPC. Sends a request to /hephaestus.event.v1.ProductEventService/WatchProject.
    pub async fn watch_project(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchProjectRequest,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchProjectResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.watch_project_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the WatchProject RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn watch_project_with_options(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchProjectRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchProjectResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_server_stream(
                &self.transport,
                &self.config,
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchProject",
                request,
                options,
            )
            .await
    }
    /// Call the WatchRepository RPC. Sends a request to /hephaestus.event.v1.ProductEventService/WatchRepository.
    pub async fn watch_repository(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchRepositoryRequest,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchRepositoryResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.watch_repository_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the WatchRepository RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn watch_repository_with_options(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchRepositoryRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchRepositoryResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_server_stream(
                &self.transport,
                &self.config,
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchRepository",
                request,
                options,
            )
            .await
    }
    /// Call the WatchRun RPC. Sends a request to /hephaestus.event.v1.ProductEventService/WatchRun.
    pub async fn watch_run(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchRunRequest,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchRunResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.watch_run_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the WatchRun RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn watch_run_with_options(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchRunRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchRunResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_server_stream(
                &self.transport,
                &self.config,
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchRun",
                request,
                options,
            )
            .await
    }
    /// Call the WatchAgentInstance RPC. Sends a request to /hephaestus.event.v1.ProductEventService/WatchAgentInstance.
    pub async fn watch_agent_instance(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchAgentInstanceRequest,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchAgentInstanceResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.watch_agent_instance_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the WatchAgentInstance RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn watch_agent_instance_with_options(
        &self,
        request: crate::messages::hephaestus::event::v1::WatchAgentInstanceRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::event::v1::__buffa::view::WatchAgentInstanceResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_server_stream(
                &self.transport,
                &self.config,
                PRODUCT_EVENT_SERVICE_SERVICE_NAME,
                "WatchAgentInstance",
                request,
                options,
            )
            .await
    }
}
