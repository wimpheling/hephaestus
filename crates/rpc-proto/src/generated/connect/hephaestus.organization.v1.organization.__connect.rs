///Shorthand for `OwnedView<ListOrganizationsRequestView<'static>>`.
pub type OwnedListOrganizationsRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationsRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListOrganizationsResponseView<'static>>`.
pub type OwnedListOrganizationsResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationsResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetOrganizationRequestView<'static>>`.
pub type OwnedGetOrganizationRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::GetOrganizationRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetOrganizationResponseView<'static>>`.
pub type OwnedGetOrganizationResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::GetOrganizationResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListOrganizationRepositoriesRequestView<'static>>`.
pub type OwnedListOrganizationRepositoriesRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationRepositoriesRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListOrganizationRepositoriesResponseView<'static>>`.
pub type OwnedListOrganizationRepositoriesResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationRepositoriesResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListOrganizationProjectsRequestView<'static>>`.
pub type OwnedListOrganizationProjectsRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationProjectsRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListOrganizationProjectsResponseView<'static>>`.
pub type OwnedListOrganizationProjectsResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationProjectsResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::organization::v1::ListOrganizationsResponse,
>
for crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationsResponseView<
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
    crate::messages::hephaestus::organization::v1::ListOrganizationsResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationsResponseView<
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
    crate::messages::hephaestus::organization::v1::GetOrganizationResponse,
>
for crate::messages::hephaestus::organization::v1::__buffa::view::GetOrganizationResponseView<
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
    crate::messages::hephaestus::organization::v1::GetOrganizationResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::GetOrganizationResponseView<
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
    crate::messages::hephaestus::organization::v1::ListOrganizationRepositoriesResponse,
>
for crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationRepositoriesResponseView<
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
    crate::messages::hephaestus::organization::v1::ListOrganizationRepositoriesResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationRepositoriesResponseView<
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
    crate::messages::hephaestus::organization::v1::ListOrganizationProjectsResponse,
>
for crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationProjectsResponseView<
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
    crate::messages::hephaestus::organization::v1::ListOrganizationProjectsResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationProjectsResponseView<
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
pub const ORGANIZATION_SERVICE_SERVICE_NAME: &str = "hephaestus.organization.v1.OrganizationService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListOrganizations` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const ORGANIZATION_SERVICE_LIST_ORGANIZATIONS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.organization.v1.OrganizationService/ListOrganizations",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetOrganization` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const ORGANIZATION_SERVICE_GET_ORGANIZATION_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.organization.v1.OrganizationService/GetOrganization",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListOrganizationRepositories` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const ORGANIZATION_SERVICE_LIST_ORGANIZATION_REPOSITORIES_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.organization.v1.OrganizationService/ListOrganizationRepositories",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListOrganizationProjects` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const ORGANIZATION_SERVICE_LIST_ORGANIZATION_PROJECTS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.organization.v1.OrganizationService/ListOrganizationProjects",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Server trait for OrganizationService.
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
pub trait OrganizationService: Send + Sync + 'static {
    /// Handle the ListOrganizations RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_organizations<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::organization::v1::ListOrganizationsRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::organization::v1::ListOrganizationsResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the GetOrganization RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_organization<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::organization::v1::GetOrganizationRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::organization::v1::GetOrganizationResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ListOrganizationRepositories RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_organization_repositories<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::organization::v1::ListOrganizationRepositoriesRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::organization::v1::ListOrganizationRepositoriesResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ListOrganizationProjects RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_organization_projects<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::organization::v1::ListOrganizationProjectsRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::organization::v1::ListOrganizationProjectsResponse,
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
pub trait OrganizationServiceExt: OrganizationService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: OrganizationService> OrganizationServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view_idempotent(
                ORGANIZATION_SERVICE_SERVICE_NAME,
                "ListOrganizations",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationsRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::organization::v1::ListOrganizationsRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_organizations(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::organization::v1::ListOrganizationsResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(ORGANIZATION_SERVICE_LIST_ORGANIZATIONS_SPEC)
            .route_view_idempotent(
                ORGANIZATION_SERVICE_SERVICE_NAME,
                "GetOrganization",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::organization::v1::__buffa::view::GetOrganizationRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::organization::v1::GetOrganizationRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_organization(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::organization::v1::GetOrganizationResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(ORGANIZATION_SERVICE_GET_ORGANIZATION_SPEC)
            .route_view_idempotent(
                ORGANIZATION_SERVICE_SERVICE_NAME,
                "ListOrganizationRepositories",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationRepositoriesRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::organization::v1::ListOrganizationRepositoriesRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_organization_repositories(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::organization::v1::ListOrganizationRepositoriesResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(ORGANIZATION_SERVICE_LIST_ORGANIZATION_REPOSITORIES_SPEC)
            .route_view_idempotent(
                ORGANIZATION_SERVICE_SERVICE_NAME,
                "ListOrganizationProjects",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationProjectsRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::organization::v1::ListOrganizationProjectsRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_organization_projects(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::organization::v1::ListOrganizationProjectsResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(ORGANIZATION_SERVICE_LIST_ORGANIZATION_PROJECTS_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct OrganizationServiceRegisterMarker;
impl<
    S: OrganizationService,
> ::connectrpc::ServiceRegister<OrganizationServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as OrganizationServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `OrganizationService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = OrganizationServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct OrganizationServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: OrganizationService> OrganizationServiceServer<T> {
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
impl<T> Clone for OrganizationServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: OrganizationService> ::connectrpc::Dispatcher for OrganizationServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path
            .strip_prefix("hephaestus.organization.v1.OrganizationService/")?;
        match method {
            "ListOrganizations" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(ORGANIZATION_SERVICE_LIST_ORGANIZATIONS_SPEC),
                )
            }
            "GetOrganization" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(ORGANIZATION_SERVICE_GET_ORGANIZATION_SPEC),
                )
            }
            "ListOrganizationRepositories" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(
                            ORGANIZATION_SERVICE_LIST_ORGANIZATION_REPOSITORIES_SPEC,
                        ),
                )
            }
            "ListOrganizationProjects" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(ORGANIZATION_SERVICE_LIST_ORGANIZATION_PROJECTS_SPEC),
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
            .strip_prefix("hephaestus.organization.v1.OrganizationService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "ListOrganizations" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::organization::v1::ListOrganizationsRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationsRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::organization::v1::ListOrganizationsRequest,
                    >::from_parts(&req, &body);
                    svc.list_organizations(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::organization::v1::ListOrganizationsResponse,
                        >(format)
                })
            }
            "GetOrganization" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::organization::v1::GetOrganizationRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::organization::v1::__buffa::view::GetOrganizationRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::organization::v1::GetOrganizationRequest,
                    >::from_parts(&req, &body);
                    svc.get_organization(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::organization::v1::GetOrganizationResponse,
                        >(format)
                })
            }
            "ListOrganizationRepositories" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::organization::v1::ListOrganizationRepositoriesRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationRepositoriesRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::organization::v1::ListOrganizationRepositoriesRequest,
                    >::from_parts(&req, &body);
                    svc.list_organization_repositories(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::organization::v1::ListOrganizationRepositoriesResponse,
                        >(format)
                })
            }
            "ListOrganizationProjects" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::organization::v1::ListOrganizationProjectsRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationProjectsRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::organization::v1::ListOrganizationProjectsRequest,
                    >::from_parts(&req, &body);
                    svc.list_organization_projects(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::organization::v1::ListOrganizationProjectsResponse,
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
            .strip_prefix("hephaestus.organization.v1.OrganizationService/") else {
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
            .strip_prefix("hephaestus.organization.v1.OrganizationService/") else {
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
            .strip_prefix("hephaestus.organization.v1.OrganizationService/") else {
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
/// let client = OrganizationServiceClient::new(conn, config);
/// let response = client.list_organizations(request).await?;
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
/// let client = OrganizationServiceClient::new(http, config);
/// let response = client.list_organizations(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.list_organizations(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.list_organizations(request).await?.into_owned();
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
pub struct OrganizationServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> OrganizationServiceClient<T>
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
    /// Call the ListOrganizations RPC. Sends a request to /hephaestus.organization.v1.OrganizationService/ListOrganizations.
    pub async fn list_organizations(
        &self,
        request: crate::messages::hephaestus::organization::v1::ListOrganizationsRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_organizations_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListOrganizations RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_organizations_with_options(
        &self,
        request: crate::messages::hephaestus::organization::v1::ListOrganizationsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                ORGANIZATION_SERVICE_SERVICE_NAME,
                "ListOrganizations",
                request,
                options,
            )
            .await
    }
    /// Call the GetOrganization RPC. Sends a request to /hephaestus.organization.v1.OrganizationService/GetOrganization.
    pub async fn get_organization(
        &self,
        request: crate::messages::hephaestus::organization::v1::GetOrganizationRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::organization::v1::__buffa::view::GetOrganizationResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_organization_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the GetOrganization RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_organization_with_options(
        &self,
        request: crate::messages::hephaestus::organization::v1::GetOrganizationRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::organization::v1::__buffa::view::GetOrganizationResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                ORGANIZATION_SERVICE_SERVICE_NAME,
                "GetOrganization",
                request,
                options,
            )
            .await
    }
    /// Call the ListOrganizationRepositories RPC. Sends a request to /hephaestus.organization.v1.OrganizationService/ListOrganizationRepositories.
    pub async fn list_organization_repositories(
        &self,
        request: crate::messages::hephaestus::organization::v1::ListOrganizationRepositoriesRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationRepositoriesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_organization_repositories_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListOrganizationRepositories RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_organization_repositories_with_options(
        &self,
        request: crate::messages::hephaestus::organization::v1::ListOrganizationRepositoriesRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationRepositoriesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                ORGANIZATION_SERVICE_SERVICE_NAME,
                "ListOrganizationRepositories",
                request,
                options,
            )
            .await
    }
    /// Call the ListOrganizationProjects RPC. Sends a request to /hephaestus.organization.v1.OrganizationService/ListOrganizationProjects.
    pub async fn list_organization_projects(
        &self,
        request: crate::messages::hephaestus::organization::v1::ListOrganizationProjectsRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationProjectsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_organization_projects_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListOrganizationProjects RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_organization_projects_with_options(
        &self,
        request: crate::messages::hephaestus::organization::v1::ListOrganizationProjectsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::organization::v1::__buffa::view::ListOrganizationProjectsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                ORGANIZATION_SERVICE_SERVICE_NAME,
                "ListOrganizationProjects",
                request,
                options,
            )
            .await
    }
}
