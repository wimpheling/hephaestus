///Shorthand for `OwnedView<ListPersonalAccessTokensRequestView<'static>>`.
pub type OwnedListPersonalAccessTokensRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::ListPersonalAccessTokensRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListPersonalAccessTokensResponseView<'static>>`.
pub type OwnedListPersonalAccessTokensResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::ListPersonalAccessTokensResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreatePersonalAccessTokenRequestView<'static>>`.
pub type OwnedCreatePersonalAccessTokenRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::CreatePersonalAccessTokenRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreatePersonalAccessTokenResponseView<'static>>`.
pub type OwnedCreatePersonalAccessTokenResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::CreatePersonalAccessTokenResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RotatePersonalAccessTokenRequestView<'static>>`.
pub type OwnedRotatePersonalAccessTokenRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::RotatePersonalAccessTokenRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RotatePersonalAccessTokenResponseView<'static>>`.
pub type OwnedRotatePersonalAccessTokenResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::RotatePersonalAccessTokenResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RevokePersonalAccessTokenRequestView<'static>>`.
pub type OwnedRevokePersonalAccessTokenRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::RevokePersonalAccessTokenRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RevokePersonalAccessTokenResponseView<'static>>`.
pub type OwnedRevokePersonalAccessTokenResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::RevokePersonalAccessTokenResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::pat::v1::ListPersonalAccessTokensResponse,
>
for crate::messages::hephaestus::pat::v1::__buffa::view::ListPersonalAccessTokensResponseView<
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
    crate::messages::hephaestus::pat::v1::ListPersonalAccessTokensResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::ListPersonalAccessTokensResponseView<
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
    crate::messages::hephaestus::pat::v1::CreatePersonalAccessTokenResponse,
>
for crate::messages::hephaestus::pat::v1::__buffa::view::CreatePersonalAccessTokenResponseView<
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
    crate::messages::hephaestus::pat::v1::CreatePersonalAccessTokenResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::CreatePersonalAccessTokenResponseView<
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
    crate::messages::hephaestus::pat::v1::RotatePersonalAccessTokenResponse,
>
for crate::messages::hephaestus::pat::v1::__buffa::view::RotatePersonalAccessTokenResponseView<
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
    crate::messages::hephaestus::pat::v1::RotatePersonalAccessTokenResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::RotatePersonalAccessTokenResponseView<
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
    crate::messages::hephaestus::pat::v1::RevokePersonalAccessTokenResponse,
>
for crate::messages::hephaestus::pat::v1::__buffa::view::RevokePersonalAccessTokenResponseView<
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
    crate::messages::hephaestus::pat::v1::RevokePersonalAccessTokenResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::pat::v1::__buffa::view::RevokePersonalAccessTokenResponseView<
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
pub const PERSONAL_ACCESS_TOKEN_SERVICE_SERVICE_NAME: &str = "hephaestus.pat.v1.PersonalAccessTokenService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListPersonalAccessTokens` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PERSONAL_ACCESS_TOKEN_SERVICE_LIST_PERSONAL_ACCESS_TOKENS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.pat.v1.PersonalAccessTokenService/ListPersonalAccessTokens",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `CreatePersonalAccessToken` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PERSONAL_ACCESS_TOKEN_SERVICE_CREATE_PERSONAL_ACCESS_TOKEN_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.pat.v1.PersonalAccessTokenService/CreatePersonalAccessToken",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `RotatePersonalAccessToken` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PERSONAL_ACCESS_TOKEN_SERVICE_ROTATE_PERSONAL_ACCESS_TOKEN_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.pat.v1.PersonalAccessTokenService/RotatePersonalAccessToken",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `RevokePersonalAccessToken` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const PERSONAL_ACCESS_TOKEN_SERVICE_REVOKE_PERSONAL_ACCESS_TOKEN_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.pat.v1.PersonalAccessTokenService/RevokePersonalAccessToken",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Manages developer-owned Git personal access tokens. Browser authentication
/// remains OIDC; these credentials are accepted only at the Git boundary.
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
pub trait PersonalAccessTokenService: Send + Sync + 'static {
    /// Handle the ListPersonalAccessTokens RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_personal_access_tokens<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::pat::v1::ListPersonalAccessTokensRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::pat::v1::ListPersonalAccessTokensResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the CreatePersonalAccessToken RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn create_personal_access_token<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::pat::v1::CreatePersonalAccessTokenRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::pat::v1::CreatePersonalAccessTokenResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the RotatePersonalAccessToken RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn rotate_personal_access_token<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::pat::v1::RotatePersonalAccessTokenRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::pat::v1::RotatePersonalAccessTokenResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the RevokePersonalAccessToken RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn revoke_personal_access_token<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::pat::v1::RevokePersonalAccessTokenRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::pat::v1::RevokePersonalAccessTokenResponse,
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
pub trait PersonalAccessTokenServiceExt: PersonalAccessTokenService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: PersonalAccessTokenService> PersonalAccessTokenServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view_idempotent(
                PERSONAL_ACCESS_TOKEN_SERVICE_SERVICE_NAME,
                "ListPersonalAccessTokens",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::pat::v1::__buffa::view::ListPersonalAccessTokensRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::pat::v1::ListPersonalAccessTokensRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_personal_access_tokens(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::pat::v1::ListPersonalAccessTokensResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(PERSONAL_ACCESS_TOKEN_SERVICE_LIST_PERSONAL_ACCESS_TOKENS_SPEC)
            .route_view(
                PERSONAL_ACCESS_TOKEN_SERVICE_SERVICE_NAME,
                "CreatePersonalAccessToken",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::pat::v1::__buffa::view::CreatePersonalAccessTokenRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::pat::v1::CreatePersonalAccessTokenRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.create_personal_access_token(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::pat::v1::CreatePersonalAccessTokenResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(PERSONAL_ACCESS_TOKEN_SERVICE_CREATE_PERSONAL_ACCESS_TOKEN_SPEC)
            .route_view(
                PERSONAL_ACCESS_TOKEN_SERVICE_SERVICE_NAME,
                "RotatePersonalAccessToken",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::pat::v1::__buffa::view::RotatePersonalAccessTokenRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::pat::v1::RotatePersonalAccessTokenRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.rotate_personal_access_token(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::pat::v1::RotatePersonalAccessTokenResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(PERSONAL_ACCESS_TOKEN_SERVICE_ROTATE_PERSONAL_ACCESS_TOKEN_SPEC)
            .route_view(
                PERSONAL_ACCESS_TOKEN_SERVICE_SERVICE_NAME,
                "RevokePersonalAccessToken",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::pat::v1::__buffa::view::RevokePersonalAccessTokenRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::pat::v1::RevokePersonalAccessTokenRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.revoke_personal_access_token(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::pat::v1::RevokePersonalAccessTokenResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(PERSONAL_ACCESS_TOKEN_SERVICE_REVOKE_PERSONAL_ACCESS_TOKEN_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct PersonalAccessTokenServiceRegisterMarker;
impl<
    S: PersonalAccessTokenService,
> ::connectrpc::ServiceRegister<PersonalAccessTokenServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as PersonalAccessTokenServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `PersonalAccessTokenService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = PersonalAccessTokenServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct PersonalAccessTokenServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: PersonalAccessTokenService> PersonalAccessTokenServiceServer<T> {
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
impl<T> Clone for PersonalAccessTokenServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: PersonalAccessTokenService> ::connectrpc::Dispatcher
for PersonalAccessTokenServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("hephaestus.pat.v1.PersonalAccessTokenService/")?;
        match method {
            "ListPersonalAccessTokens" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(
                            PERSONAL_ACCESS_TOKEN_SERVICE_LIST_PERSONAL_ACCESS_TOKENS_SPEC,
                        ),
                )
            }
            "CreatePersonalAccessToken" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(
                            PERSONAL_ACCESS_TOKEN_SERVICE_CREATE_PERSONAL_ACCESS_TOKEN_SPEC,
                        ),
                )
            }
            "RotatePersonalAccessToken" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(
                            PERSONAL_ACCESS_TOKEN_SERVICE_ROTATE_PERSONAL_ACCESS_TOKEN_SPEC,
                        ),
                )
            }
            "RevokePersonalAccessToken" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(
                            PERSONAL_ACCESS_TOKEN_SERVICE_REVOKE_PERSONAL_ACCESS_TOKEN_SPEC,
                        ),
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
            .strip_prefix("hephaestus.pat.v1.PersonalAccessTokenService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "ListPersonalAccessTokens" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::pat::v1::ListPersonalAccessTokensRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::pat::v1::__buffa::view::ListPersonalAccessTokensRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::pat::v1::ListPersonalAccessTokensRequest,
                    >::from_parts(&req, &body);
                    svc.list_personal_access_tokens(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::pat::v1::ListPersonalAccessTokensResponse,
                        >(format)
                })
            }
            "CreatePersonalAccessToken" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::pat::v1::CreatePersonalAccessTokenRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::pat::v1::__buffa::view::CreatePersonalAccessTokenRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::pat::v1::CreatePersonalAccessTokenRequest,
                    >::from_parts(&req, &body);
                    svc.create_personal_access_token(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::pat::v1::CreatePersonalAccessTokenResponse,
                        >(format)
                })
            }
            "RotatePersonalAccessToken" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::pat::v1::RotatePersonalAccessTokenRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::pat::v1::__buffa::view::RotatePersonalAccessTokenRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::pat::v1::RotatePersonalAccessTokenRequest,
                    >::from_parts(&req, &body);
                    svc.rotate_personal_access_token(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::pat::v1::RotatePersonalAccessTokenResponse,
                        >(format)
                })
            }
            "RevokePersonalAccessToken" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::pat::v1::RevokePersonalAccessTokenRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::pat::v1::__buffa::view::RevokePersonalAccessTokenRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::pat::v1::RevokePersonalAccessTokenRequest,
                    >::from_parts(&req, &body);
                    svc.revoke_personal_access_token(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::pat::v1::RevokePersonalAccessTokenResponse,
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
            .strip_prefix("hephaestus.pat.v1.PersonalAccessTokenService/") else {
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
            .strip_prefix("hephaestus.pat.v1.PersonalAccessTokenService/") else {
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
            .strip_prefix("hephaestus.pat.v1.PersonalAccessTokenService/") else {
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
/// let client = PersonalAccessTokenServiceClient::new(conn, config);
/// let response = client.list_personal_access_tokens(request).await?;
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
/// let client = PersonalAccessTokenServiceClient::new(http, config);
/// let response = client.list_personal_access_tokens(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.list_personal_access_tokens(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.list_personal_access_tokens(request).await?.into_owned();
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
pub struct PersonalAccessTokenServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> PersonalAccessTokenServiceClient<T>
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
    /// Call the ListPersonalAccessTokens RPC. Sends a request to /hephaestus.pat.v1.PersonalAccessTokenService/ListPersonalAccessTokens.
    pub async fn list_personal_access_tokens(
        &self,
        request: crate::messages::hephaestus::pat::v1::ListPersonalAccessTokensRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::pat::v1::__buffa::view::ListPersonalAccessTokensResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_personal_access_tokens_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListPersonalAccessTokens RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_personal_access_tokens_with_options(
        &self,
        request: crate::messages::hephaestus::pat::v1::ListPersonalAccessTokensRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::pat::v1::__buffa::view::ListPersonalAccessTokensResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                PERSONAL_ACCESS_TOKEN_SERVICE_SERVICE_NAME,
                "ListPersonalAccessTokens",
                request,
                options,
            )
            .await
    }
    /// Call the CreatePersonalAccessToken RPC. Sends a request to /hephaestus.pat.v1.PersonalAccessTokenService/CreatePersonalAccessToken.
    pub async fn create_personal_access_token(
        &self,
        request: crate::messages::hephaestus::pat::v1::CreatePersonalAccessTokenRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::pat::v1::__buffa::view::CreatePersonalAccessTokenResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.create_personal_access_token_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the CreatePersonalAccessToken RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn create_personal_access_token_with_options(
        &self,
        request: crate::messages::hephaestus::pat::v1::CreatePersonalAccessTokenRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::pat::v1::__buffa::view::CreatePersonalAccessTokenResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                PERSONAL_ACCESS_TOKEN_SERVICE_SERVICE_NAME,
                "CreatePersonalAccessToken",
                request,
                options,
            )
            .await
    }
    /// Call the RotatePersonalAccessToken RPC. Sends a request to /hephaestus.pat.v1.PersonalAccessTokenService/RotatePersonalAccessToken.
    pub async fn rotate_personal_access_token(
        &self,
        request: crate::messages::hephaestus::pat::v1::RotatePersonalAccessTokenRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::pat::v1::__buffa::view::RotatePersonalAccessTokenResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.rotate_personal_access_token_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the RotatePersonalAccessToken RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn rotate_personal_access_token_with_options(
        &self,
        request: crate::messages::hephaestus::pat::v1::RotatePersonalAccessTokenRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::pat::v1::__buffa::view::RotatePersonalAccessTokenResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                PERSONAL_ACCESS_TOKEN_SERVICE_SERVICE_NAME,
                "RotatePersonalAccessToken",
                request,
                options,
            )
            .await
    }
    /// Call the RevokePersonalAccessToken RPC. Sends a request to /hephaestus.pat.v1.PersonalAccessTokenService/RevokePersonalAccessToken.
    pub async fn revoke_personal_access_token(
        &self,
        request: crate::messages::hephaestus::pat::v1::RevokePersonalAccessTokenRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::pat::v1::__buffa::view::RevokePersonalAccessTokenResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.revoke_personal_access_token_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the RevokePersonalAccessToken RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn revoke_personal_access_token_with_options(
        &self,
        request: crate::messages::hephaestus::pat::v1::RevokePersonalAccessTokenRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::pat::v1::__buffa::view::RevokePersonalAccessTokenResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                PERSONAL_ACCESS_TOKEN_SERVICE_SERVICE_NAME,
                "RevokePersonalAccessToken",
                request,
                options,
            )
            .await
    }
}
