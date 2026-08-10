///Shorthand for `OwnedView<CreateProjectRequestView<'static>>`.
pub type OwnedCreateProjectRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::CreateProjectRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreateProjectResponseView<'static>>`.
pub type OwnedCreateProjectResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::CreateProjectResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetProjectRequestView<'static>>`.
pub type OwnedGetProjectRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::GetProjectRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetProjectResponseView<'static>>`.
pub type OwnedGetProjectResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::GetProjectResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListProjectRepositoriesRequestView<'static>>`.
pub type OwnedListProjectRepositoriesRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::ListProjectRepositoriesRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListProjectRepositoriesResponseView<'static>>`.
pub type OwnedListProjectRepositoriesResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::ListProjectRepositoriesResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListProjectInstancesRequestView<'static>>`.
pub type OwnedListProjectInstancesRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::ListProjectInstancesRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListProjectInstancesResponseView<'static>>`.
pub type OwnedListProjectInstancesResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::ListProjectInstancesResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListImportableReleaseAgentsRequestView<'static>>`.
pub type OwnedListImportableReleaseAgentsRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::ListImportableReleaseAgentsRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListImportableReleaseAgentsResponseView<'static>>`.
pub type OwnedListImportableReleaseAgentsResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::ListImportableReleaseAgentsResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::project::v1::CreateProjectResponse,
>
for crate::messages::hephaestus::project::v1::__buffa::view::CreateProjectResponseView<
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
    crate::messages::hephaestus::project::v1::CreateProjectResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::CreateProjectResponseView<
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
    crate::messages::hephaestus::project::v1::GetProjectResponse,
>
for crate::messages::hephaestus::project::v1::__buffa::view::GetProjectResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::project::v1::GetProjectResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::GetProjectResponseView<
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
    crate::messages::hephaestus::project::v1::ListProjectRepositoriesResponse,
>
for crate::messages::hephaestus::project::v1::__buffa::view::ListProjectRepositoriesResponseView<
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
    crate::messages::hephaestus::project::v1::ListProjectRepositoriesResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::ListProjectRepositoriesResponseView<
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
    crate::messages::hephaestus::project::v1::ListProjectInstancesResponse,
>
for crate::messages::hephaestus::project::v1::__buffa::view::ListProjectInstancesResponseView<
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
    crate::messages::hephaestus::project::v1::ListProjectInstancesResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::ListProjectInstancesResponseView<
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
    crate::messages::hephaestus::project::v1::ListImportableReleaseAgentsResponse,
>
for crate::messages::hephaestus::project::v1::__buffa::view::ListImportableReleaseAgentsResponseView<
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
    crate::messages::hephaestus::project::v1::ListImportableReleaseAgentsResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::project::v1::__buffa::view::ListImportableReleaseAgentsResponseView<
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
pub const PROJECT_SERVICE_SERVICE_NAME: &str = "hephaestus.project.v1.ProjectService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `CreateProject` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PROJECT_SERVICE_CREATE_PROJECT_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.project.v1.ProjectService/CreateProject",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetProject` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PROJECT_SERVICE_GET_PROJECT_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.project.v1.ProjectService/GetProject",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListProjectRepositories` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PROJECT_SERVICE_LIST_PROJECT_REPOSITORIES_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.project.v1.ProjectService/ListProjectRepositories",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListProjectInstances` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PROJECT_SERVICE_LIST_PROJECT_INSTANCES_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.project.v1.ProjectService/ListProjectInstances",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListImportableReleaseAgents` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PROJECT_SERVICE_LIST_IMPORTABLE_RELEASE_AGENTS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.project.v1.ProjectService/ListImportableReleaseAgents",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Server trait for ProjectService.
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
pub trait ProjectService: Send + Sync + 'static {
    /// Handle the CreateProject RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn create_project<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::project::v1::CreateProjectRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::project::v1::CreateProjectResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the GetProject RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_project<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::project::v1::GetProjectRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::project::v1::GetProjectResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ListProjectRepositories RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_project_repositories<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::project::v1::ListProjectRepositoriesRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::project::v1::ListProjectRepositoriesResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ListProjectInstances RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_project_instances<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::project::v1::ListProjectInstancesRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::project::v1::ListProjectInstancesResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ListImportableReleaseAgents RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_importable_release_agents<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::project::v1::ListImportableReleaseAgentsRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::project::v1::ListImportableReleaseAgentsResponse,
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
pub trait ProjectServiceExt: ProjectService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: ProjectService> ProjectServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view(
                PROJECT_SERVICE_SERVICE_NAME,
                "CreateProject",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::project::v1::__buffa::view::CreateProjectRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::project::v1::CreateProjectRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.create_project(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::project::v1::CreateProjectResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(PROJECT_SERVICE_CREATE_PROJECT_SPEC)
            .route_view_idempotent(
                PROJECT_SERVICE_SERVICE_NAME,
                "GetProject",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::project::v1::__buffa::view::GetProjectRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::project::v1::GetProjectRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_project(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::project::v1::GetProjectResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(PROJECT_SERVICE_GET_PROJECT_SPEC)
            .route_view_idempotent(
                PROJECT_SERVICE_SERVICE_NAME,
                "ListProjectRepositories",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::project::v1::__buffa::view::ListProjectRepositoriesRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::project::v1::ListProjectRepositoriesRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_project_repositories(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::project::v1::ListProjectRepositoriesResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(PROJECT_SERVICE_LIST_PROJECT_REPOSITORIES_SPEC)
            .route_view_idempotent(
                PROJECT_SERVICE_SERVICE_NAME,
                "ListProjectInstances",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::project::v1::__buffa::view::ListProjectInstancesRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::project::v1::ListProjectInstancesRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_project_instances(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::project::v1::ListProjectInstancesResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(PROJECT_SERVICE_LIST_PROJECT_INSTANCES_SPEC)
            .route_view_idempotent(
                PROJECT_SERVICE_SERVICE_NAME,
                "ListImportableReleaseAgents",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::project::v1::__buffa::view::ListImportableReleaseAgentsRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::project::v1::ListImportableReleaseAgentsRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_importable_release_agents(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::project::v1::ListImportableReleaseAgentsResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(PROJECT_SERVICE_LIST_IMPORTABLE_RELEASE_AGENTS_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct ProjectServiceRegisterMarker;
impl<S: ProjectService> ::connectrpc::ServiceRegister<ProjectServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as ProjectServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `ProjectService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = ProjectServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct ProjectServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: ProjectService> ProjectServiceServer<T> {
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
impl<T> Clone for ProjectServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: ProjectService> ::connectrpc::Dispatcher for ProjectServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("hephaestus.project.v1.ProjectService/")?;
        match method {
            "CreateProject" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(PROJECT_SERVICE_CREATE_PROJECT_SPEC),
                )
            }
            "GetProject" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(PROJECT_SERVICE_GET_PROJECT_SPEC),
                )
            }
            "ListProjectRepositories" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(PROJECT_SERVICE_LIST_PROJECT_REPOSITORIES_SPEC),
                )
            }
            "ListProjectInstances" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(PROJECT_SERVICE_LIST_PROJECT_INSTANCES_SPEC),
                )
            }
            "ListImportableReleaseAgents" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(PROJECT_SERVICE_LIST_IMPORTABLE_RELEASE_AGENTS_SPEC),
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
        let Some(method) = path.strip_prefix("hephaestus.project.v1.ProjectService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "CreateProject" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::project::v1::CreateProjectRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::project::v1::__buffa::view::CreateProjectRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::project::v1::CreateProjectRequest,
                    >::from_parts(&req, &body);
                    svc.create_project(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::project::v1::CreateProjectResponse,
                        >(format)
                })
            }
            "GetProject" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::project::v1::GetProjectRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::project::v1::__buffa::view::GetProjectRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::project::v1::GetProjectRequest,
                    >::from_parts(&req, &body);
                    svc.get_project(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::project::v1::GetProjectResponse,
                        >(format)
                })
            }
            "ListProjectRepositories" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::project::v1::ListProjectRepositoriesRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::project::v1::__buffa::view::ListProjectRepositoriesRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::project::v1::ListProjectRepositoriesRequest,
                    >::from_parts(&req, &body);
                    svc.list_project_repositories(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::project::v1::ListProjectRepositoriesResponse,
                        >(format)
                })
            }
            "ListProjectInstances" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::project::v1::ListProjectInstancesRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::project::v1::__buffa::view::ListProjectInstancesRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::project::v1::ListProjectInstancesRequest,
                    >::from_parts(&req, &body);
                    svc.list_project_instances(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::project::v1::ListProjectInstancesResponse,
                        >(format)
                })
            }
            "ListImportableReleaseAgents" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::project::v1::ListImportableReleaseAgentsRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::project::v1::__buffa::view::ListImportableReleaseAgentsRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::project::v1::ListImportableReleaseAgentsRequest,
                    >::from_parts(&req, &body);
                    svc.list_importable_release_agents(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::project::v1::ListImportableReleaseAgentsResponse,
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
        let Some(method) = path.strip_prefix("hephaestus.project.v1.ProjectService/")
        else {
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
        let Some(method) = path.strip_prefix("hephaestus.project.v1.ProjectService/")
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
        let Some(method) = path.strip_prefix("hephaestus.project.v1.ProjectService/")
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
/// let client = ProjectServiceClient::new(conn, config);
/// let response = client.create_project(request).await?;
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
/// let client = ProjectServiceClient::new(http, config);
/// let response = client.create_project(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.create_project(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.create_project(request).await?.into_owned();
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
pub struct ProjectServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> ProjectServiceClient<T>
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
    /// Call the CreateProject RPC. Sends a request to /hephaestus.project.v1.ProjectService/CreateProject.
    pub async fn create_project(
        &self,
        request: crate::messages::hephaestus::project::v1::CreateProjectRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::project::v1::__buffa::view::CreateProjectResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.create_project_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the CreateProject RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn create_project_with_options(
        &self,
        request: crate::messages::hephaestus::project::v1::CreateProjectRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::project::v1::__buffa::view::CreateProjectResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                PROJECT_SERVICE_SERVICE_NAME,
                "CreateProject",
                request,
                options,
            )
            .await
    }
    /// Call the GetProject RPC. Sends a request to /hephaestus.project.v1.ProjectService/GetProject.
    pub async fn get_project(
        &self,
        request: crate::messages::hephaestus::project::v1::GetProjectRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::project::v1::__buffa::view::GetProjectResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_project_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the GetProject RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_project_with_options(
        &self,
        request: crate::messages::hephaestus::project::v1::GetProjectRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::project::v1::__buffa::view::GetProjectResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                PROJECT_SERVICE_SERVICE_NAME,
                "GetProject",
                request,
                options,
            )
            .await
    }
    /// Call the ListProjectRepositories RPC. Sends a request to /hephaestus.project.v1.ProjectService/ListProjectRepositories.
    pub async fn list_project_repositories(
        &self,
        request: crate::messages::hephaestus::project::v1::ListProjectRepositoriesRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::project::v1::__buffa::view::ListProjectRepositoriesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_project_repositories_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListProjectRepositories RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_project_repositories_with_options(
        &self,
        request: crate::messages::hephaestus::project::v1::ListProjectRepositoriesRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::project::v1::__buffa::view::ListProjectRepositoriesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                PROJECT_SERVICE_SERVICE_NAME,
                "ListProjectRepositories",
                request,
                options,
            )
            .await
    }
    /// Call the ListProjectInstances RPC. Sends a request to /hephaestus.project.v1.ProjectService/ListProjectInstances.
    pub async fn list_project_instances(
        &self,
        request: crate::messages::hephaestus::project::v1::ListProjectInstancesRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::project::v1::__buffa::view::ListProjectInstancesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_project_instances_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListProjectInstances RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_project_instances_with_options(
        &self,
        request: crate::messages::hephaestus::project::v1::ListProjectInstancesRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::project::v1::__buffa::view::ListProjectInstancesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                PROJECT_SERVICE_SERVICE_NAME,
                "ListProjectInstances",
                request,
                options,
            )
            .await
    }
    /// Call the ListImportableReleaseAgents RPC. Sends a request to /hephaestus.project.v1.ProjectService/ListImportableReleaseAgents.
    pub async fn list_importable_release_agents(
        &self,
        request: crate::messages::hephaestus::project::v1::ListImportableReleaseAgentsRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::project::v1::__buffa::view::ListImportableReleaseAgentsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_importable_release_agents_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListImportableReleaseAgents RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_importable_release_agents_with_options(
        &self,
        request: crate::messages::hephaestus::project::v1::ListImportableReleaseAgentsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::project::v1::__buffa::view::ListImportableReleaseAgentsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                PROJECT_SERVICE_SERVICE_NAME,
                "ListImportableReleaseAgents",
                request,
                options,
            )
            .await
    }
}
