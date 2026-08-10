///Shorthand for `OwnedView<ListRepositoryReleasesRequestView<'static>>`.
pub type OwnedListRepositoryReleasesRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::ListRepositoryReleasesRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListRepositoryReleasesResponseView<'static>>`.
pub type OwnedListRepositoryReleasesResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::ListRepositoryReleasesResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetReleaseRequestView<'static>>`.
pub type OwnedGetReleaseRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::GetReleaseRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetReleaseResponseView<'static>>`.
pub type OwnedGetReleaseResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::GetReleaseResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<SetDraftVersionRequestView<'static>>`.
pub type OwnedSetDraftVersionRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::SetDraftVersionRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<SetDraftVersionResponseView<'static>>`.
pub type OwnedSetDraftVersionResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::SetDraftVersionResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<PublishReleaseRequestView<'static>>`.
pub type OwnedPublishReleaseRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::PublishReleaseRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<PublishReleaseResponseView<'static>>`.
pub type OwnedPublishReleaseResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::PublishReleaseResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchReleaseRequestView<'static>>`.
pub type OwnedWatchReleaseRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::WatchReleaseRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchReleaseResponseView<'static>>`.
pub type OwnedWatchReleaseResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::WatchReleaseResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::release::v1::ListRepositoryReleasesResponse,
>
for crate::messages::hephaestus::release::v1::__buffa::view::ListRepositoryReleasesResponseView<
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
    crate::messages::hephaestus::release::v1::ListRepositoryReleasesResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::ListRepositoryReleasesResponseView<
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
    crate::messages::hephaestus::release::v1::GetReleaseResponse,
>
for crate::messages::hephaestus::release::v1::__buffa::view::GetReleaseResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::release::v1::GetReleaseResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::GetReleaseResponseView<
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
    crate::messages::hephaestus::release::v1::SetDraftVersionResponse,
>
for crate::messages::hephaestus::release::v1::__buffa::view::SetDraftVersionResponseView<
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
    crate::messages::hephaestus::release::v1::SetDraftVersionResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::SetDraftVersionResponseView<
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
    crate::messages::hephaestus::release::v1::PublishReleaseResponse,
>
for crate::messages::hephaestus::release::v1::__buffa::view::PublishReleaseResponseView<
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
    crate::messages::hephaestus::release::v1::PublishReleaseResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::PublishReleaseResponseView<
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
    crate::messages::hephaestus::release::v1::WatchReleaseResponse,
>
for crate::messages::hephaestus::release::v1::__buffa::view::WatchReleaseResponseView<
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
    crate::messages::hephaestus::release::v1::WatchReleaseResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::release::v1::__buffa::view::WatchReleaseResponseView<
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
pub const RELEASE_SERVICE_SERVICE_NAME: &str = "hephaestus.release.v1.ReleaseService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListRepositoryReleases` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const RELEASE_SERVICE_LIST_REPOSITORY_RELEASES_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.release.v1.ReleaseService/ListRepositoryReleases",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetRelease` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const RELEASE_SERVICE_GET_RELEASE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.release.v1.ReleaseService/GetRelease",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `SetDraftVersion` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const RELEASE_SERVICE_SET_DRAFT_VERSION_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.release.v1.ReleaseService/SetDraftVersion",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `PublishRelease` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const RELEASE_SERVICE_PUBLISH_RELEASE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.release.v1.ReleaseService/PublishRelease",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `WatchRelease` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const RELEASE_SERVICE_WATCH_RELEASE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.release.v1.ReleaseService/WatchRelease",
        ::connectrpc::StreamType::ServerStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Server trait for ReleaseService.
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
pub trait ReleaseService: Send + Sync + 'static {
    /// Handle the ListRepositoryReleases RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_repository_releases<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::release::v1::ListRepositoryReleasesRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::release::v1::ListRepositoryReleasesResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the GetRelease RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_release<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::release::v1::GetReleaseRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::release::v1::GetReleaseResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the SetDraftVersion RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn set_draft_version<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::release::v1::SetDraftVersionRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::release::v1::SetDraftVersionResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the PublishRelease RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn publish_release<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::release::v1::PublishReleaseRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::release::v1::PublishReleaseResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the WatchRelease RPC.
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call (until the response stream is returned);
    /// message fields are read directly on it (zero-copy). Data the
    /// returned stream needs must be copied out or converted via
    /// `.to_owned_message()`.
    fn watch_release(
        &self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::release::v1::WatchReleaseRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::messages::hephaestus::release::v1::WatchReleaseResponse,
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
pub trait ReleaseServiceExt: ReleaseService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: ReleaseService> ReleaseServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view_idempotent(
                RELEASE_SERVICE_SERVICE_NAME,
                "ListRepositoryReleases",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::release::v1::__buffa::view::ListRepositoryReleasesRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::release::v1::ListRepositoryReleasesRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_repository_releases(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::release::v1::ListRepositoryReleasesResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(RELEASE_SERVICE_LIST_REPOSITORY_RELEASES_SPEC)
            .route_view_idempotent(
                RELEASE_SERVICE_SERVICE_NAME,
                "GetRelease",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::release::v1::__buffa::view::GetReleaseRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::release::v1::GetReleaseRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_release(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::release::v1::GetReleaseResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(RELEASE_SERVICE_GET_RELEASE_SPEC)
            .route_view(
                RELEASE_SERVICE_SERVICE_NAME,
                "SetDraftVersion",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::release::v1::__buffa::view::SetDraftVersionRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::release::v1::SetDraftVersionRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.set_draft_version(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::release::v1::SetDraftVersionResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(RELEASE_SERVICE_SET_DRAFT_VERSION_SPEC)
            .route_view(
                RELEASE_SERVICE_SERVICE_NAME,
                "PublishRelease",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::release::v1::__buffa::view::PublishReleaseRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::release::v1::PublishReleaseRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.publish_release(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::release::v1::PublishReleaseResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(RELEASE_SERVICE_PUBLISH_RELEASE_SPEC)
            .route_view_server_stream::<
                _,
                _,
                crate::messages::hephaestus::release::v1::WatchReleaseResponse,
            >(
                RELEASE_SERVICE_SERVICE_NAME,
                "WatchRelease",
                ::connectrpc::view_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::release::v1::__buffa::view::WatchReleaseRequestView<
                                'static,
                            >,
                        >|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::release::v1::WatchReleaseRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.watch_release(ctx, sreq).await
                        }
                    }
                }),
            )
            .with_spec(RELEASE_SERVICE_WATCH_RELEASE_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct ReleaseServiceRegisterMarker;
impl<S: ReleaseService> ::connectrpc::ServiceRegister<ReleaseServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as ReleaseServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `ReleaseService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = ReleaseServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct ReleaseServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: ReleaseService> ReleaseServiceServer<T> {
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
impl<T> Clone for ReleaseServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: ReleaseService> ::connectrpc::Dispatcher for ReleaseServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("hephaestus.release.v1.ReleaseService/")?;
        match method {
            "ListRepositoryReleases" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(RELEASE_SERVICE_LIST_REPOSITORY_RELEASES_SPEC),
                )
            }
            "GetRelease" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(RELEASE_SERVICE_GET_RELEASE_SPEC),
                )
            }
            "SetDraftVersion" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(RELEASE_SERVICE_SET_DRAFT_VERSION_SPEC),
                )
            }
            "PublishRelease" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(RELEASE_SERVICE_PUBLISH_RELEASE_SPEC),
                )
            }
            "WatchRelease" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::server_streaming()
                        .with_spec(RELEASE_SERVICE_WATCH_RELEASE_SPEC),
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
        let Some(method) = path.strip_prefix("hephaestus.release.v1.ReleaseService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "ListRepositoryReleases" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::release::v1::ListRepositoryReleasesRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::release::v1::__buffa::view::ListRepositoryReleasesRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::release::v1::ListRepositoryReleasesRequest,
                    >::from_parts(&req, &body);
                    svc.list_repository_releases(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::release::v1::ListRepositoryReleasesResponse,
                        >(format)
                })
            }
            "GetRelease" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::release::v1::GetReleaseRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::release::v1::__buffa::view::GetReleaseRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::release::v1::GetReleaseRequest,
                    >::from_parts(&req, &body);
                    svc.get_release(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::release::v1::GetReleaseResponse,
                        >(format)
                })
            }
            "SetDraftVersion" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::release::v1::SetDraftVersionRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::release::v1::__buffa::view::SetDraftVersionRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::release::v1::SetDraftVersionRequest,
                    >::from_parts(&req, &body);
                    svc.set_draft_version(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::release::v1::SetDraftVersionResponse,
                        >(format)
                })
            }
            "PublishRelease" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::release::v1::PublishReleaseRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::release::v1::__buffa::view::PublishReleaseRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::release::v1::PublishReleaseRequest,
                    >::from_parts(&req, &body);
                    svc.publish_release(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::release::v1::PublishReleaseResponse,
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
        let Some(method) = path.strip_prefix("hephaestus.release.v1.ReleaseService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_streaming(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "WatchRelease" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::release::v1::WatchReleaseRequest,
                    >(request, format)?;
                    let req: crate::messages::hephaestus::release::v1::__buffa::view::WatchReleaseRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::release::v1::WatchReleaseRequest,
                    >::from_parts(&req, &body);
                    let resp = svc.watch_release(ctx, req).await?;
                    Ok(
                        resp
                            .map_body(|s| ::connectrpc::dispatcher::codegen::encode_response_stream::<
                                crate::messages::hephaestus::release::v1::WatchReleaseResponse,
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
        let Some(method) = path.strip_prefix("hephaestus.release.v1.ReleaseService/")
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
        let Some(method) = path.strip_prefix("hephaestus.release.v1.ReleaseService/")
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
/// let client = ReleaseServiceClient::new(conn, config);
/// let response = client.list_repository_releases(request).await?;
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
/// let client = ReleaseServiceClient::new(http, config);
/// let response = client.list_repository_releases(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.list_repository_releases(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.list_repository_releases(request).await?.into_owned();
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
pub struct ReleaseServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> ReleaseServiceClient<T>
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
    /// Call the ListRepositoryReleases RPC. Sends a request to /hephaestus.release.v1.ReleaseService/ListRepositoryReleases.
    pub async fn list_repository_releases(
        &self,
        request: crate::messages::hephaestus::release::v1::ListRepositoryReleasesRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::release::v1::__buffa::view::ListRepositoryReleasesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_repository_releases_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListRepositoryReleases RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_repository_releases_with_options(
        &self,
        request: crate::messages::hephaestus::release::v1::ListRepositoryReleasesRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::release::v1::__buffa::view::ListRepositoryReleasesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                RELEASE_SERVICE_SERVICE_NAME,
                "ListRepositoryReleases",
                request,
                options,
            )
            .await
    }
    /// Call the GetRelease RPC. Sends a request to /hephaestus.release.v1.ReleaseService/GetRelease.
    pub async fn get_release(
        &self,
        request: crate::messages::hephaestus::release::v1::GetReleaseRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::release::v1::__buffa::view::GetReleaseResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_release_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the GetRelease RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_release_with_options(
        &self,
        request: crate::messages::hephaestus::release::v1::GetReleaseRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::release::v1::__buffa::view::GetReleaseResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                RELEASE_SERVICE_SERVICE_NAME,
                "GetRelease",
                request,
                options,
            )
            .await
    }
    /// Call the SetDraftVersion RPC. Sends a request to /hephaestus.release.v1.ReleaseService/SetDraftVersion.
    pub async fn set_draft_version(
        &self,
        request: crate::messages::hephaestus::release::v1::SetDraftVersionRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::release::v1::__buffa::view::SetDraftVersionResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.set_draft_version_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the SetDraftVersion RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn set_draft_version_with_options(
        &self,
        request: crate::messages::hephaestus::release::v1::SetDraftVersionRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::release::v1::__buffa::view::SetDraftVersionResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                RELEASE_SERVICE_SERVICE_NAME,
                "SetDraftVersion",
                request,
                options,
            )
            .await
    }
    /// Call the PublishRelease RPC. Sends a request to /hephaestus.release.v1.ReleaseService/PublishRelease.
    pub async fn publish_release(
        &self,
        request: crate::messages::hephaestus::release::v1::PublishReleaseRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::release::v1::__buffa::view::PublishReleaseResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.publish_release_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the PublishRelease RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn publish_release_with_options(
        &self,
        request: crate::messages::hephaestus::release::v1::PublishReleaseRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::release::v1::__buffa::view::PublishReleaseResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                RELEASE_SERVICE_SERVICE_NAME,
                "PublishRelease",
                request,
                options,
            )
            .await
    }
    /// Call the WatchRelease RPC. Sends a request to /hephaestus.release.v1.ReleaseService/WatchRelease.
    pub async fn watch_release(
        &self,
        request: crate::messages::hephaestus::release::v1::WatchReleaseRequest,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::release::v1::__buffa::view::WatchReleaseResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.watch_release_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the WatchRelease RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn watch_release_with_options(
        &self,
        request: crate::messages::hephaestus::release::v1::WatchReleaseRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::release::v1::__buffa::view::WatchReleaseResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_server_stream(
                &self.transport,
                &self.config,
                RELEASE_SERVICE_SERVICE_NAME,
                "WatchRelease",
                request,
                options,
            )
            .await
    }
}
