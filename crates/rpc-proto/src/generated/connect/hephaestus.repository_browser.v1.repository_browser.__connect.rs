///Shorthand for `OwnedView<ListBranchesRequestView<'static>>`.
pub type OwnedListBranchesRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListBranchesRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListBranchesResponseView<'static>>`.
pub type OwnedListBranchesResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListBranchesResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListCommitsRequestView<'static>>`.
pub type OwnedListCommitsRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListCommitsRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListCommitsResponseView<'static>>`.
pub type OwnedListCommitsResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListCommitsResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetTreeRequestView<'static>>`.
pub type OwnedGetTreeRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetTreeRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetTreeResponseView<'static>>`.
pub type OwnedGetTreeResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetTreeResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetFileRequestView<'static>>`.
pub type OwnedGetFileRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetFileRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetFileResponseView<'static>>`.
pub type OwnedGetFileResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetFileResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<StreamFileRequestView<'static>>`.
pub type OwnedStreamFileRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::StreamFileRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<StreamFileResponseView<'static>>`.
pub type OwnedStreamFileResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::StreamFileResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::repository_browser::v1::ListBranchesResponse,
>
for crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListBranchesResponseView<
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
    crate::messages::hephaestus::repository_browser::v1::ListBranchesResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListBranchesResponseView<
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
    crate::messages::hephaestus::repository_browser::v1::ListCommitsResponse,
>
for crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListCommitsResponseView<
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
    crate::messages::hephaestus::repository_browser::v1::ListCommitsResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListCommitsResponseView<
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
    crate::messages::hephaestus::repository_browser::v1::GetTreeResponse,
>
for crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetTreeResponseView<
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
    crate::messages::hephaestus::repository_browser::v1::GetTreeResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetTreeResponseView<
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
    crate::messages::hephaestus::repository_browser::v1::GetFileResponse,
>
for crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetFileResponseView<
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
    crate::messages::hephaestus::repository_browser::v1::GetFileResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetFileResponseView<
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
    crate::messages::hephaestus::repository_browser::v1::StreamFileResponse,
>
for crate::messages::hephaestus::repository_browser::v1::__buffa::view::StreamFileResponseView<
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
    crate::messages::hephaestus::repository_browser::v1::StreamFileResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::repository_browser::v1::__buffa::view::StreamFileResponseView<
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
pub const REPOSITORY_BROWSER_SERVICE_SERVICE_NAME: &str = "hephaestus.repository_browser.v1.RepositoryBrowserService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListBranches` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const REPOSITORY_BROWSER_SERVICE_LIST_BRANCHES_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/ListBranches",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListCommits` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const REPOSITORY_BROWSER_SERVICE_LIST_COMMITS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/ListCommits",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetTree` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const REPOSITORY_BROWSER_SERVICE_GET_TREE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/GetTree",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetFile` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const REPOSITORY_BROWSER_SERVICE_GET_FILE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/GetFile",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `StreamFile` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const REPOSITORY_BROWSER_SERVICE_STREAM_FILE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.repository_browser.v1.RepositoryBrowserService/StreamFile",
        ::connectrpc::StreamType::ServerStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Server trait for RepositoryBrowserService.
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
pub trait RepositoryBrowserService: Send + Sync + 'static {
    /// Handle the ListBranches RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_branches<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::repository_browser::v1::ListBranchesRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::repository_browser::v1::ListBranchesResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ListCommits RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_commits<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::repository_browser::v1::ListCommitsRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::repository_browser::v1::ListCommitsResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the GetTree RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_tree<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::repository_browser::v1::GetTreeRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::repository_browser::v1::GetTreeResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the GetFile RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_file<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::repository_browser::v1::GetFileRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::repository_browser::v1::GetFileResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the StreamFile RPC.
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call (until the response stream is returned);
    /// message fields are read directly on it (zero-copy). Data the
    /// returned stream needs must be copied out or converted via
    /// `.to_owned_message()`.
    fn stream_file(
        &self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::repository_browser::v1::StreamFileRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::messages::hephaestus::repository_browser::v1::StreamFileResponse,
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
pub trait RepositoryBrowserServiceExt: RepositoryBrowserService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: RepositoryBrowserService> RepositoryBrowserServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view_idempotent(
                REPOSITORY_BROWSER_SERVICE_SERVICE_NAME,
                "ListBranches",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListBranchesRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::repository_browser::v1::ListBranchesRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_branches(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::repository_browser::v1::ListBranchesResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(REPOSITORY_BROWSER_SERVICE_LIST_BRANCHES_SPEC)
            .route_view_idempotent(
                REPOSITORY_BROWSER_SERVICE_SERVICE_NAME,
                "ListCommits",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListCommitsRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::repository_browser::v1::ListCommitsRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_commits(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::repository_browser::v1::ListCommitsResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(REPOSITORY_BROWSER_SERVICE_LIST_COMMITS_SPEC)
            .route_view_idempotent(
                REPOSITORY_BROWSER_SERVICE_SERVICE_NAME,
                "GetTree",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetTreeRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::repository_browser::v1::GetTreeRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_tree(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::repository_browser::v1::GetTreeResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(REPOSITORY_BROWSER_SERVICE_GET_TREE_SPEC)
            .route_view_idempotent(
                REPOSITORY_BROWSER_SERVICE_SERVICE_NAME,
                "GetFile",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetFileRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::repository_browser::v1::GetFileRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_file(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::repository_browser::v1::GetFileResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(REPOSITORY_BROWSER_SERVICE_GET_FILE_SPEC)
            .route_view_server_stream::<
                _,
                _,
                crate::messages::hephaestus::repository_browser::v1::StreamFileResponse,
            >(
                REPOSITORY_BROWSER_SERVICE_SERVICE_NAME,
                "StreamFile",
                ::connectrpc::view_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::repository_browser::v1::__buffa::view::StreamFileRequestView<
                                'static,
                            >,
                        >|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::repository_browser::v1::StreamFileRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.stream_file(ctx, sreq).await
                        }
                    }
                }),
            )
            .with_spec(REPOSITORY_BROWSER_SERVICE_STREAM_FILE_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct RepositoryBrowserServiceRegisterMarker;
impl<
    S: RepositoryBrowserService,
> ::connectrpc::ServiceRegister<RepositoryBrowserServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as RepositoryBrowserServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `RepositoryBrowserService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = RepositoryBrowserServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct RepositoryBrowserServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: RepositoryBrowserService> RepositoryBrowserServiceServer<T> {
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
impl<T> Clone for RepositoryBrowserServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: RepositoryBrowserService> ::connectrpc::Dispatcher
for RepositoryBrowserServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path
            .strip_prefix("hephaestus.repository_browser.v1.RepositoryBrowserService/")?;
        match method {
            "ListBranches" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(REPOSITORY_BROWSER_SERVICE_LIST_BRANCHES_SPEC),
                )
            }
            "ListCommits" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(REPOSITORY_BROWSER_SERVICE_LIST_COMMITS_SPEC),
                )
            }
            "GetTree" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(REPOSITORY_BROWSER_SERVICE_GET_TREE_SPEC),
                )
            }
            "GetFile" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(REPOSITORY_BROWSER_SERVICE_GET_FILE_SPEC),
                )
            }
            "StreamFile" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::server_streaming()
                        .with_spec(REPOSITORY_BROWSER_SERVICE_STREAM_FILE_SPEC),
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
            .strip_prefix("hephaestus.repository_browser.v1.RepositoryBrowserService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "ListBranches" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::repository_browser::v1::ListBranchesRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListBranchesRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::repository_browser::v1::ListBranchesRequest,
                    >::from_parts(&req, &body);
                    svc.list_branches(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::repository_browser::v1::ListBranchesResponse,
                        >(format)
                })
            }
            "ListCommits" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::repository_browser::v1::ListCommitsRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListCommitsRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::repository_browser::v1::ListCommitsRequest,
                    >::from_parts(&req, &body);
                    svc.list_commits(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::repository_browser::v1::ListCommitsResponse,
                        >(format)
                })
            }
            "GetTree" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::repository_browser::v1::GetTreeRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetTreeRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::repository_browser::v1::GetTreeRequest,
                    >::from_parts(&req, &body);
                    svc.get_tree(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::repository_browser::v1::GetTreeResponse,
                        >(format)
                })
            }
            "GetFile" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::repository_browser::v1::GetFileRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetFileRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::repository_browser::v1::GetFileRequest,
                    >::from_parts(&req, &body);
                    svc.get_file(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::repository_browser::v1::GetFileResponse,
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
            .strip_prefix("hephaestus.repository_browser.v1.RepositoryBrowserService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_streaming(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "StreamFile" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::repository_browser::v1::StreamFileRequest,
                    >(request, format)?;
                    let req: crate::messages::hephaestus::repository_browser::v1::__buffa::view::StreamFileRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::repository_browser::v1::StreamFileRequest,
                    >::from_parts(&req, &body);
                    let resp = svc.stream_file(ctx, req).await?;
                    Ok(
                        resp
                            .map_body(|s| ::connectrpc::dispatcher::codegen::encode_response_stream::<
                                crate::messages::hephaestus::repository_browser::v1::StreamFileResponse,
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
        let Some(method) = path
            .strip_prefix("hephaestus.repository_browser.v1.RepositoryBrowserService/")
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
        let Some(method) = path
            .strip_prefix("hephaestus.repository_browser.v1.RepositoryBrowserService/")
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
/// let client = RepositoryBrowserServiceClient::new(conn, config);
/// let response = client.list_branches(request).await?;
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
/// let client = RepositoryBrowserServiceClient::new(http, config);
/// let response = client.list_branches(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.list_branches(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.list_branches(request).await?.into_owned();
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
pub struct RepositoryBrowserServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> RepositoryBrowserServiceClient<T>
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
    /// Call the ListBranches RPC. Sends a request to /hephaestus.repository_browser.v1.RepositoryBrowserService/ListBranches.
    pub async fn list_branches(
        &self,
        request: crate::messages::hephaestus::repository_browser::v1::ListBranchesRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListBranchesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_branches_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListBranches RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_branches_with_options(
        &self,
        request: crate::messages::hephaestus::repository_browser::v1::ListBranchesRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListBranchesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                REPOSITORY_BROWSER_SERVICE_SERVICE_NAME,
                "ListBranches",
                request,
                options,
            )
            .await
    }
    /// Call the ListCommits RPC. Sends a request to /hephaestus.repository_browser.v1.RepositoryBrowserService/ListCommits.
    pub async fn list_commits(
        &self,
        request: crate::messages::hephaestus::repository_browser::v1::ListCommitsRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListCommitsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_commits_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListCommits RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_commits_with_options(
        &self,
        request: crate::messages::hephaestus::repository_browser::v1::ListCommitsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository_browser::v1::__buffa::view::ListCommitsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                REPOSITORY_BROWSER_SERVICE_SERVICE_NAME,
                "ListCommits",
                request,
                options,
            )
            .await
    }
    /// Call the GetTree RPC. Sends a request to /hephaestus.repository_browser.v1.RepositoryBrowserService/GetTree.
    pub async fn get_tree(
        &self,
        request: crate::messages::hephaestus::repository_browser::v1::GetTreeRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetTreeResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_tree_with_options(request, ::connectrpc::client::CallOptions::default())
            .await
    }
    /// Call the GetTree RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_tree_with_options(
        &self,
        request: crate::messages::hephaestus::repository_browser::v1::GetTreeRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetTreeResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                REPOSITORY_BROWSER_SERVICE_SERVICE_NAME,
                "GetTree",
                request,
                options,
            )
            .await
    }
    /// Call the GetFile RPC. Sends a request to /hephaestus.repository_browser.v1.RepositoryBrowserService/GetFile.
    pub async fn get_file(
        &self,
        request: crate::messages::hephaestus::repository_browser::v1::GetFileRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetFileResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_file_with_options(request, ::connectrpc::client::CallOptions::default())
            .await
    }
    /// Call the GetFile RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_file_with_options(
        &self,
        request: crate::messages::hephaestus::repository_browser::v1::GetFileRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::repository_browser::v1::__buffa::view::GetFileResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                REPOSITORY_BROWSER_SERVICE_SERVICE_NAME,
                "GetFile",
                request,
                options,
            )
            .await
    }
    /// Call the StreamFile RPC. Sends a request to /hephaestus.repository_browser.v1.RepositoryBrowserService/StreamFile.
    pub async fn stream_file(
        &self,
        request: crate::messages::hephaestus::repository_browser::v1::StreamFileRequest,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::repository_browser::v1::__buffa::view::StreamFileResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.stream_file_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the StreamFile RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn stream_file_with_options(
        &self,
        request: crate::messages::hephaestus::repository_browser::v1::StreamFileRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::repository_browser::v1::__buffa::view::StreamFileResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_server_stream(
                &self.transport,
                &self.config,
                REPOSITORY_BROWSER_SERVICE_SERVICE_NAME,
                "StreamFile",
                request,
                options,
            )
            .await
    }
}
