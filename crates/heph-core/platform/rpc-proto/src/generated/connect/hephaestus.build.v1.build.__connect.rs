///Shorthand for `OwnedView<ListBuildsRequestView<'static>>`.
pub type OwnedListBuildsRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::ListBuildsRequestView<'static>,
>;
///Shorthand for `OwnedView<ListBuildsResponseView<'static>>`.
pub type OwnedListBuildsResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::ListBuildsResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetBuildRequestView<'static>>`.
pub type OwnedGetBuildRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::GetBuildRequestView<'static>,
>;
///Shorthand for `OwnedView<GetBuildResponseView<'static>>`.
pub type OwnedGetBuildResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::GetBuildResponseView<'static>,
>;
///Shorthand for `OwnedView<RequestBuildRequestView<'static>>`.
pub type OwnedRequestBuildRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::RequestBuildRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RequestBuildResponseView<'static>>`.
pub type OwnedRequestBuildResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::RequestBuildResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RetryBuildRequestView<'static>>`.
pub type OwnedRetryBuildRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::RetryBuildRequestView<'static>,
>;
///Shorthand for `OwnedView<RetryBuildResponseView<'static>>`.
pub type OwnedRetryBuildResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::RetryBuildResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RebuildForVerificationRequestView<'static>>`.
pub type OwnedRebuildForVerificationRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::RebuildForVerificationRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RebuildForVerificationResponseView<'static>>`.
pub type OwnedRebuildForVerificationResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::RebuildForVerificationResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchBuildRequestView<'static>>`.
pub type OwnedWatchBuildRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::WatchBuildRequestView<'static>,
>;
///Shorthand for `OwnedView<WatchBuildResponseView<'static>>`.
pub type OwnedWatchBuildResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::WatchBuildResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchRepositoryBuildsRequestView<'static>>`.
pub type OwnedWatchRepositoryBuildsRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::WatchRepositoryBuildsRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<WatchRepositoryBuildsResponseView<'static>>`.
pub type OwnedWatchRepositoryBuildsResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::WatchRepositoryBuildsResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<StreamBuildLogsRequestView<'static>>`.
pub type OwnedStreamBuildLogsRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::StreamBuildLogsRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<StreamBuildLogsResponseView<'static>>`.
pub type OwnedStreamBuildLogsResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::StreamBuildLogsResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<crate::messages::hephaestus::build::v1::ListBuildsResponse>
for crate::messages::hephaestus::build::v1::__buffa::view::ListBuildsResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<crate::messages::hephaestus::build::v1::ListBuildsResponse>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::ListBuildsResponseView<
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
impl ::connectrpc::Encodable<crate::messages::hephaestus::build::v1::GetBuildResponse>
for crate::messages::hephaestus::build::v1::__buffa::view::GetBuildResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<crate::messages::hephaestus::build::v1::GetBuildResponse>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::GetBuildResponseView<'static>,
> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self.reborrow(), codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::build::v1::RequestBuildResponse,
>
for crate::messages::hephaestus::build::v1::__buffa::view::RequestBuildResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::build::v1::RequestBuildResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::RequestBuildResponseView<
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
impl ::connectrpc::Encodable<crate::messages::hephaestus::build::v1::RetryBuildResponse>
for crate::messages::hephaestus::build::v1::__buffa::view::RetryBuildResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<crate::messages::hephaestus::build::v1::RetryBuildResponse>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::RetryBuildResponseView<
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
    crate::messages::hephaestus::build::v1::RebuildForVerificationResponse,
>
for crate::messages::hephaestus::build::v1::__buffa::view::RebuildForVerificationResponseView<
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
    crate::messages::hephaestus::build::v1::RebuildForVerificationResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::RebuildForVerificationResponseView<
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
impl ::connectrpc::Encodable<crate::messages::hephaestus::build::v1::WatchBuildResponse>
for crate::messages::hephaestus::build::v1::__buffa::view::WatchBuildResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<crate::messages::hephaestus::build::v1::WatchBuildResponse>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::WatchBuildResponseView<
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
    crate::messages::hephaestus::build::v1::WatchRepositoryBuildsResponse,
>
for crate::messages::hephaestus::build::v1::__buffa::view::WatchRepositoryBuildsResponseView<
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
    crate::messages::hephaestus::build::v1::WatchRepositoryBuildsResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::WatchRepositoryBuildsResponseView<
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
    crate::messages::hephaestus::build::v1::StreamBuildLogsResponse,
>
for crate::messages::hephaestus::build::v1::__buffa::view::StreamBuildLogsResponseView<
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
    crate::messages::hephaestus::build::v1::StreamBuildLogsResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::build::v1::__buffa::view::StreamBuildLogsResponseView<
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
pub const BUILD_SERVICE_SERVICE_NAME: &str = "hephaestus.build.v1.BuildService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListBuilds` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const BUILD_SERVICE_LIST_BUILDS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.build.v1.BuildService/ListBuilds",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetBuild` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const BUILD_SERVICE_GET_BUILD_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.build.v1.BuildService/GetBuild",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `RequestBuild` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const BUILD_SERVICE_REQUEST_BUILD_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.build.v1.BuildService/RequestBuild",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `RetryBuild` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const BUILD_SERVICE_RETRY_BUILD_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.build.v1.BuildService/RetryBuild",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `RebuildForVerification` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const BUILD_SERVICE_REBUILD_FOR_VERIFICATION_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.build.v1.BuildService/RebuildForVerification",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `WatchBuild` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const BUILD_SERVICE_WATCH_BUILD_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.build.v1.BuildService/WatchBuild",
        ::connectrpc::StreamType::ServerStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `WatchRepositoryBuilds` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const BUILD_SERVICE_WATCH_REPOSITORY_BUILDS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.build.v1.BuildService/WatchRepositoryBuilds",
        ::connectrpc::StreamType::ServerStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `StreamBuildLogs` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const BUILD_SERVICE_STREAM_BUILD_LOGS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.build.v1.BuildService/StreamBuildLogs",
        ::connectrpc::StreamType::ServerStream,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Server trait for BuildService.
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
pub trait BuildService: Send + Sync + 'static {
    /// Handle the ListBuilds RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_builds<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::build::v1::ListBuildsRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::build::v1::ListBuildsResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the GetBuild RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_build<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::build::v1::GetBuildRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::build::v1::GetBuildResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the RequestBuild RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn request_build<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::build::v1::RequestBuildRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::build::v1::RequestBuildResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the RetryBuild RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn retry_build<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::build::v1::RetryBuildRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::build::v1::RetryBuildResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the RebuildForVerification RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn rebuild_for_verification<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::build::v1::RebuildForVerificationRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::build::v1::RebuildForVerificationResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the WatchBuild RPC.
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call (until the response stream is returned);
    /// message fields are read directly on it (zero-copy). Data the
    /// returned stream needs must be copied out or converted via
    /// `.to_owned_message()`.
    fn watch_build(
        &self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::build::v1::WatchBuildRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::messages::hephaestus::build::v1::WatchBuildResponse,
                > + Send + use<Self>,
            >,
        >,
    > + Send;
    /// Handle the WatchRepositoryBuilds RPC.
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call (until the response stream is returned);
    /// message fields are read directly on it (zero-copy). Data the
    /// returned stream needs must be copied out or converted via
    /// `.to_owned_message()`.
    fn watch_repository_builds(
        &self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::build::v1::WatchRepositoryBuildsRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::messages::hephaestus::build::v1::WatchRepositoryBuildsResponse,
                > + Send + use<Self>,
            >,
        >,
    > + Send;
    /// Handle the StreamBuildLogs RPC.
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call (until the response stream is returned);
    /// message fields are read directly on it (zero-copy). Data the
    /// returned stream needs must be copied out or converted via
    /// `.to_owned_message()`.
    fn stream_build_logs(
        &self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::build::v1::StreamBuildLogsRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            ::connectrpc::ServiceStream<
                impl ::connectrpc::Encodable<
                    crate::messages::hephaestus::build::v1::StreamBuildLogsResponse,
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
pub trait BuildServiceExt: BuildService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: BuildService> BuildServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view_idempotent(
                BUILD_SERVICE_SERVICE_NAME,
                "ListBuilds",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::build::v1::__buffa::view::ListBuildsRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::build::v1::ListBuildsRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_builds(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::build::v1::ListBuildsResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(BUILD_SERVICE_LIST_BUILDS_SPEC)
            .route_view_idempotent(
                BUILD_SERVICE_SERVICE_NAME,
                "GetBuild",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::build::v1::__buffa::view::GetBuildRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::build::v1::GetBuildRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_build(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::build::v1::GetBuildResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(BUILD_SERVICE_GET_BUILD_SPEC)
            .route_view(
                BUILD_SERVICE_SERVICE_NAME,
                "RequestBuild",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::build::v1::__buffa::view::RequestBuildRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::build::v1::RequestBuildRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.request_build(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::build::v1::RequestBuildResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(BUILD_SERVICE_REQUEST_BUILD_SPEC)
            .route_view(
                BUILD_SERVICE_SERVICE_NAME,
                "RetryBuild",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::build::v1::__buffa::view::RetryBuildRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::build::v1::RetryBuildRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.retry_build(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::build::v1::RetryBuildResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(BUILD_SERVICE_RETRY_BUILD_SPEC)
            .route_view(
                BUILD_SERVICE_SERVICE_NAME,
                "RebuildForVerification",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::build::v1::__buffa::view::RebuildForVerificationRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::build::v1::RebuildForVerificationRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.rebuild_for_verification(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::build::v1::RebuildForVerificationResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(BUILD_SERVICE_REBUILD_FOR_VERIFICATION_SPEC)
            .route_view_server_stream::<
                _,
                _,
                crate::messages::hephaestus::build::v1::WatchBuildResponse,
            >(
                BUILD_SERVICE_SERVICE_NAME,
                "WatchBuild",
                ::connectrpc::view_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::build::v1::__buffa::view::WatchBuildRequestView<
                                'static,
                            >,
                        >|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::build::v1::WatchBuildRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.watch_build(ctx, sreq).await
                        }
                    }
                }),
            )
            .with_spec(BUILD_SERVICE_WATCH_BUILD_SPEC)
            .route_view_server_stream::<
                _,
                _,
                crate::messages::hephaestus::build::v1::WatchRepositoryBuildsResponse,
            >(
                BUILD_SERVICE_SERVICE_NAME,
                "WatchRepositoryBuilds",
                ::connectrpc::view_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::build::v1::__buffa::view::WatchRepositoryBuildsRequestView<
                                'static,
                            >,
                        >|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::build::v1::WatchRepositoryBuildsRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.watch_repository_builds(ctx, sreq).await
                        }
                    }
                }),
            )
            .with_spec(BUILD_SERVICE_WATCH_REPOSITORY_BUILDS_SPEC)
            .route_view_server_stream::<
                _,
                _,
                crate::messages::hephaestus::build::v1::StreamBuildLogsResponse,
            >(
                BUILD_SERVICE_SERVICE_NAME,
                "StreamBuildLogs",
                ::connectrpc::view_streaming_handler_fn({
                    let svc = ::std::sync::Arc::clone(&self);
                    move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::build::v1::__buffa::view::StreamBuildLogsRequestView<
                                'static,
                            >,
                        >|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::build::v1::StreamBuildLogsRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.stream_build_logs(ctx, sreq).await
                        }
                    }
                }),
            )
            .with_spec(BUILD_SERVICE_STREAM_BUILD_LOGS_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct BuildServiceRegisterMarker;
impl<S: BuildService> ::connectrpc::ServiceRegister<BuildServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as BuildServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `BuildService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = BuildServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct BuildServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: BuildService> BuildServiceServer<T> {
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
impl<T> Clone for BuildServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: BuildService> ::connectrpc::Dispatcher for BuildServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("hephaestus.build.v1.BuildService/")?;
        match method {
            "ListBuilds" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(BUILD_SERVICE_LIST_BUILDS_SPEC),
                )
            }
            "GetBuild" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(BUILD_SERVICE_GET_BUILD_SPEC),
                )
            }
            "RequestBuild" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(BUILD_SERVICE_REQUEST_BUILD_SPEC),
                )
            }
            "RetryBuild" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(BUILD_SERVICE_RETRY_BUILD_SPEC),
                )
            }
            "RebuildForVerification" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(BUILD_SERVICE_REBUILD_FOR_VERIFICATION_SPEC),
                )
            }
            "WatchBuild" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::server_streaming()
                        .with_spec(BUILD_SERVICE_WATCH_BUILD_SPEC),
                )
            }
            "WatchRepositoryBuilds" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::server_streaming()
                        .with_spec(BUILD_SERVICE_WATCH_REPOSITORY_BUILDS_SPEC),
                )
            }
            "StreamBuildLogs" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::server_streaming()
                        .with_spec(BUILD_SERVICE_STREAM_BUILD_LOGS_SPEC),
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
        let Some(method) = path.strip_prefix("hephaestus.build.v1.BuildService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "ListBuilds" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::build::v1::ListBuildsRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::build::v1::__buffa::view::ListBuildsRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::build::v1::ListBuildsRequest,
                    >::from_parts(&req, &body);
                    svc.list_builds(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::build::v1::ListBuildsResponse,
                        >(format)
                })
            }
            "GetBuild" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::build::v1::GetBuildRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::build::v1::__buffa::view::GetBuildRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::build::v1::GetBuildRequest,
                    >::from_parts(&req, &body);
                    svc.get_build(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::build::v1::GetBuildResponse,
                        >(format)
                })
            }
            "RequestBuild" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::build::v1::RequestBuildRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::build::v1::__buffa::view::RequestBuildRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::build::v1::RequestBuildRequest,
                    >::from_parts(&req, &body);
                    svc.request_build(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::build::v1::RequestBuildResponse,
                        >(format)
                })
            }
            "RetryBuild" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::build::v1::RetryBuildRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::build::v1::__buffa::view::RetryBuildRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::build::v1::RetryBuildRequest,
                    >::from_parts(&req, &body);
                    svc.retry_build(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::build::v1::RetryBuildResponse,
                        >(format)
                })
            }
            "RebuildForVerification" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::build::v1::RebuildForVerificationRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::build::v1::__buffa::view::RebuildForVerificationRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::build::v1::RebuildForVerificationRequest,
                    >::from_parts(&req, &body);
                    svc.rebuild_for_verification(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::build::v1::RebuildForVerificationResponse,
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
        let Some(method) = path.strip_prefix("hephaestus.build.v1.BuildService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_streaming(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "WatchBuild" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::build::v1::WatchBuildRequest,
                    >(request, format)?;
                    let req: crate::messages::hephaestus::build::v1::__buffa::view::WatchBuildRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::build::v1::WatchBuildRequest,
                    >::from_parts(&req, &body);
                    let resp = svc.watch_build(ctx, req).await?;
                    Ok(
                        resp
                            .map_body(|s| ::connectrpc::dispatcher::codegen::encode_response_stream::<
                                crate::messages::hephaestus::build::v1::WatchBuildResponse,
                                _,
                                _,
                            >(s, format)),
                    )
                })
            }
            "WatchRepositoryBuilds" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::build::v1::WatchRepositoryBuildsRequest,
                    >(request, format)?;
                    let req: crate::messages::hephaestus::build::v1::__buffa::view::WatchRepositoryBuildsRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::build::v1::WatchRepositoryBuildsRequest,
                    >::from_parts(&req, &body);
                    let resp = svc.watch_repository_builds(ctx, req).await?;
                    Ok(
                        resp
                            .map_body(|s| ::connectrpc::dispatcher::codegen::encode_response_stream::<
                                crate::messages::hephaestus::build::v1::WatchRepositoryBuildsResponse,
                                _,
                                _,
                            >(s, format)),
                    )
                })
            }
            "StreamBuildLogs" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::build::v1::StreamBuildLogsRequest,
                    >(request, format)?;
                    let req: crate::messages::hephaestus::build::v1::__buffa::view::StreamBuildLogsRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::build::v1::StreamBuildLogsRequest,
                    >::from_parts(&req, &body);
                    let resp = svc.stream_build_logs(ctx, req).await?;
                    Ok(
                        resp
                            .map_body(|s| ::connectrpc::dispatcher::codegen::encode_response_stream::<
                                crate::messages::hephaestus::build::v1::StreamBuildLogsResponse,
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
        let Some(method) = path.strip_prefix("hephaestus.build.v1.BuildService/") else {
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
        let Some(method) = path.strip_prefix("hephaestus.build.v1.BuildService/") else {
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
/// let client = BuildServiceClient::new(conn, config);
/// let response = client.list_builds(request).await?;
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
/// let client = BuildServiceClient::new(http, config);
/// let response = client.list_builds(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.list_builds(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.list_builds(request).await?.into_owned();
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
pub struct BuildServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> BuildServiceClient<T>
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
    /// Call the ListBuilds RPC. Sends a request to /hephaestus.build.v1.BuildService/ListBuilds.
    pub async fn list_builds(
        &self,
        request: crate::messages::hephaestus::build::v1::ListBuildsRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::build::v1::__buffa::view::ListBuildsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_builds_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListBuilds RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_builds_with_options(
        &self,
        request: crate::messages::hephaestus::build::v1::ListBuildsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::build::v1::__buffa::view::ListBuildsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                BUILD_SERVICE_SERVICE_NAME,
                "ListBuilds",
                request,
                options,
            )
            .await
    }
    /// Call the GetBuild RPC. Sends a request to /hephaestus.build.v1.BuildService/GetBuild.
    pub async fn get_build(
        &self,
        request: crate::messages::hephaestus::build::v1::GetBuildRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::build::v1::__buffa::view::GetBuildResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_build_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the GetBuild RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_build_with_options(
        &self,
        request: crate::messages::hephaestus::build::v1::GetBuildRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::build::v1::__buffa::view::GetBuildResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                BUILD_SERVICE_SERVICE_NAME,
                "GetBuild",
                request,
                options,
            )
            .await
    }
    /// Call the RequestBuild RPC. Sends a request to /hephaestus.build.v1.BuildService/RequestBuild.
    pub async fn request_build(
        &self,
        request: crate::messages::hephaestus::build::v1::RequestBuildRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::build::v1::__buffa::view::RequestBuildResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.request_build_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the RequestBuild RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn request_build_with_options(
        &self,
        request: crate::messages::hephaestus::build::v1::RequestBuildRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::build::v1::__buffa::view::RequestBuildResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                BUILD_SERVICE_SERVICE_NAME,
                "RequestBuild",
                request,
                options,
            )
            .await
    }
    /// Call the RetryBuild RPC. Sends a request to /hephaestus.build.v1.BuildService/RetryBuild.
    pub async fn retry_build(
        &self,
        request: crate::messages::hephaestus::build::v1::RetryBuildRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::build::v1::__buffa::view::RetryBuildResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.retry_build_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the RetryBuild RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn retry_build_with_options(
        &self,
        request: crate::messages::hephaestus::build::v1::RetryBuildRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::build::v1::__buffa::view::RetryBuildResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                BUILD_SERVICE_SERVICE_NAME,
                "RetryBuild",
                request,
                options,
            )
            .await
    }
    /// Call the RebuildForVerification RPC. Sends a request to /hephaestus.build.v1.BuildService/RebuildForVerification.
    pub async fn rebuild_for_verification(
        &self,
        request: crate::messages::hephaestus::build::v1::RebuildForVerificationRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::build::v1::__buffa::view::RebuildForVerificationResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.rebuild_for_verification_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the RebuildForVerification RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn rebuild_for_verification_with_options(
        &self,
        request: crate::messages::hephaestus::build::v1::RebuildForVerificationRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::build::v1::__buffa::view::RebuildForVerificationResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                BUILD_SERVICE_SERVICE_NAME,
                "RebuildForVerification",
                request,
                options,
            )
            .await
    }
    /// Call the WatchBuild RPC. Sends a request to /hephaestus.build.v1.BuildService/WatchBuild.
    pub async fn watch_build(
        &self,
        request: crate::messages::hephaestus::build::v1::WatchBuildRequest,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::build::v1::__buffa::view::WatchBuildResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.watch_build_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the WatchBuild RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn watch_build_with_options(
        &self,
        request: crate::messages::hephaestus::build::v1::WatchBuildRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::build::v1::__buffa::view::WatchBuildResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_server_stream(
                &self.transport,
                &self.config,
                BUILD_SERVICE_SERVICE_NAME,
                "WatchBuild",
                request,
                options,
            )
            .await
    }
    /// Call the WatchRepositoryBuilds RPC. Sends a request to /hephaestus.build.v1.BuildService/WatchRepositoryBuilds.
    pub async fn watch_repository_builds(
        &self,
        request: crate::messages::hephaestus::build::v1::WatchRepositoryBuildsRequest,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::build::v1::__buffa::view::WatchRepositoryBuildsResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.watch_repository_builds_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the WatchRepositoryBuilds RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn watch_repository_builds_with_options(
        &self,
        request: crate::messages::hephaestus::build::v1::WatchRepositoryBuildsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::build::v1::__buffa::view::WatchRepositoryBuildsResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_server_stream(
                &self.transport,
                &self.config,
                BUILD_SERVICE_SERVICE_NAME,
                "WatchRepositoryBuilds",
                request,
                options,
            )
            .await
    }
    /// Call the StreamBuildLogs RPC. Sends a request to /hephaestus.build.v1.BuildService/StreamBuildLogs.
    pub async fn stream_build_logs(
        &self,
        request: crate::messages::hephaestus::build::v1::StreamBuildLogsRequest,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::build::v1::__buffa::view::StreamBuildLogsResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.stream_build_logs_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the StreamBuildLogs RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn stream_build_logs_with_options(
        &self,
        request: crate::messages::hephaestus::build::v1::StreamBuildLogsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::ServerStream<
            T::ResponseBody,
            crate::messages::hephaestus::build::v1::__buffa::view::StreamBuildLogsResponseView<
                'static,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_server_stream(
                &self.transport,
                &self.config,
                BUILD_SERVICE_SERVICE_NAME,
                "StreamBuildLogs",
                request,
                options,
            )
            .await
    }
}
