///Shorthand for `OwnedView<ListProjectGatewaysRequestView<'static>>`.
pub type OwnedListProjectGatewaysRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListProjectGatewaysRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListProjectGatewaysResponseView<'static>>`.
pub type OwnedListProjectGatewaysResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListProjectGatewaysResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetGatewayRequestView<'static>>`.
pub type OwnedGetGatewayRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::GetGatewayRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetGatewayResponseView<'static>>`.
pub type OwnedGetGatewayResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::GetGatewayResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<InstallReleaseGatewaysRequestView<'static>>`.
pub type OwnedInstallReleaseGatewaysRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::InstallReleaseGatewaysRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<InstallReleaseGatewaysResponseView<'static>>`.
pub type OwnedInstallReleaseGatewaysResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::InstallReleaseGatewaysResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ConfigureGatewayRequestView<'static>>`.
pub type OwnedConfigureGatewayRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ConfigureGatewayRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ConfigureGatewayResponseView<'static>>`.
pub type OwnedConfigureGatewayResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ConfigureGatewayResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListGatewayIngressRequestView<'static>>`.
pub type OwnedListGatewayIngressRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListGatewayIngressRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListGatewayIngressResponseView<'static>>`.
pub type OwnedListGatewayIngressResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListGatewayIngressResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<SetGatewayLifecycleRequestView<'static>>`.
pub type OwnedSetGatewayLifecycleRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::SetGatewayLifecycleRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<SetGatewayLifecycleResponseView<'static>>`.
pub type OwnedSetGatewayLifecycleResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::SetGatewayLifecycleResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreateMailboxBindingRequestView<'static>>`.
pub type OwnedCreateMailboxBindingRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::CreateMailboxBindingRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreateMailboxBindingResponseView<'static>>`.
pub type OwnedCreateMailboxBindingResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::CreateMailboxBindingResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RevokeMailboxBindingGrantRequestView<'static>>`.
pub type OwnedRevokeMailboxBindingGrantRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::RevokeMailboxBindingGrantRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RevokeMailboxBindingGrantResponseView<'static>>`.
pub type OwnedRevokeMailboxBindingGrantResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::RevokeMailboxBindingGrantResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListMailboxBindingsRequestView<'static>>`.
pub type OwnedListMailboxBindingsRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxBindingsRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListMailboxBindingsResponseView<'static>>`.
pub type OwnedListMailboxBindingsResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxBindingsResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListMailboxPublicationsRequestView<'static>>`.
pub type OwnedListMailboxPublicationsRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxPublicationsRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ListMailboxPublicationsResponseView<'static>>`.
pub type OwnedListMailboxPublicationsResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxPublicationsResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::gateway::v1::ListProjectGatewaysResponse,
>
for crate::messages::hephaestus::gateway::v1::__buffa::view::ListProjectGatewaysResponseView<
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
    crate::messages::hephaestus::gateway::v1::ListProjectGatewaysResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListProjectGatewaysResponseView<
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
    crate::messages::hephaestus::gateway::v1::GetGatewayResponse,
>
for crate::messages::hephaestus::gateway::v1::__buffa::view::GetGatewayResponseView<'_> {
    fn encode(
        &self,
        codec: ::connectrpc::CodecFormat,
    ) -> ::std::result::Result<::buffa::bytes::Bytes, ::connectrpc::ConnectError> {
        ::connectrpc::__codegen::encode_view_body(self, codec)
    }
}
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::gateway::v1::GetGatewayResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::GetGatewayResponseView<
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
    crate::messages::hephaestus::gateway::v1::InstallReleaseGatewaysResponse,
>
for crate::messages::hephaestus::gateway::v1::__buffa::view::InstallReleaseGatewaysResponseView<
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
    crate::messages::hephaestus::gateway::v1::InstallReleaseGatewaysResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::InstallReleaseGatewaysResponseView<
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
    crate::messages::hephaestus::gateway::v1::ConfigureGatewayResponse,
>
for crate::messages::hephaestus::gateway::v1::__buffa::view::ConfigureGatewayResponseView<
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
    crate::messages::hephaestus::gateway::v1::ConfigureGatewayResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ConfigureGatewayResponseView<
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
    crate::messages::hephaestus::gateway::v1::ListGatewayIngressResponse,
>
for crate::messages::hephaestus::gateway::v1::__buffa::view::ListGatewayIngressResponseView<
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
    crate::messages::hephaestus::gateway::v1::ListGatewayIngressResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListGatewayIngressResponseView<
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
    crate::messages::hephaestus::gateway::v1::SetGatewayLifecycleResponse,
>
for crate::messages::hephaestus::gateway::v1::__buffa::view::SetGatewayLifecycleResponseView<
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
    crate::messages::hephaestus::gateway::v1::SetGatewayLifecycleResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::SetGatewayLifecycleResponseView<
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
    crate::messages::hephaestus::gateway::v1::CreateMailboxBindingResponse,
>
for crate::messages::hephaestus::gateway::v1::__buffa::view::CreateMailboxBindingResponseView<
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
    crate::messages::hephaestus::gateway::v1::CreateMailboxBindingResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::CreateMailboxBindingResponseView<
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
    crate::messages::hephaestus::gateway::v1::RevokeMailboxBindingGrantResponse,
>
for crate::messages::hephaestus::gateway::v1::__buffa::view::RevokeMailboxBindingGrantResponseView<
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
    crate::messages::hephaestus::gateway::v1::RevokeMailboxBindingGrantResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::RevokeMailboxBindingGrantResponseView<
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
    crate::messages::hephaestus::gateway::v1::ListMailboxBindingsResponse,
>
for crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxBindingsResponseView<
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
    crate::messages::hephaestus::gateway::v1::ListMailboxBindingsResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxBindingsResponseView<
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
    crate::messages::hephaestus::gateway::v1::ListMailboxPublicationsResponse,
>
for crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxPublicationsResponseView<
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
    crate::messages::hephaestus::gateway::v1::ListMailboxPublicationsResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxPublicationsResponseView<
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
pub const GATEWAY_SERVICE_SERVICE_NAME: &str = "hephaestus.gateway.v1.GatewayService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListProjectGateways` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const GATEWAY_SERVICE_LIST_PROJECT_GATEWAYS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.gateway.v1.GatewayService/ListProjectGateways",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetGateway` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const GATEWAY_SERVICE_GET_GATEWAY_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.gateway.v1.GatewayService/GetGateway",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `InstallReleaseGateways` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const GATEWAY_SERVICE_INSTALL_RELEASE_GATEWAYS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.gateway.v1.GatewayService/InstallReleaseGateways",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ConfigureGateway` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const GATEWAY_SERVICE_CONFIGURE_GATEWAY_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.gateway.v1.GatewayService/ConfigureGateway",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListGatewayIngress` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const GATEWAY_SERVICE_LIST_GATEWAY_INGRESS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.gateway.v1.GatewayService/ListGatewayIngress",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `SetGatewayLifecycle` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const GATEWAY_SERVICE_SET_GATEWAY_LIFECYCLE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.gateway.v1.GatewayService/SetGatewayLifecycle",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `CreateMailboxBinding` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const GATEWAY_SERVICE_CREATE_MAILBOX_BINDING_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.gateway.v1.GatewayService/CreateMailboxBinding",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `RevokeMailboxBindingGrant` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const GATEWAY_SERVICE_REVOKE_MAILBOX_BINDING_GRANT_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.gateway.v1.GatewayService/RevokeMailboxBindingGrant",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListMailboxBindings` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const GATEWAY_SERVICE_LIST_MAILBOX_BINDINGS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.gateway.v1.GatewayService/ListMailboxBindings",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ListMailboxPublications` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const GATEWAY_SERVICE_LIST_MAILBOX_PUBLICATIONS_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.gateway.v1.GatewayService/ListMailboxPublications",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Project-scoped management and redacted ingress inspection for repository
/// gateways. Public request payloads and provider credentials are never part of
/// this API.
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
pub trait GatewayService: Send + Sync + 'static {
    /// Handle the ListProjectGateways RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_project_gateways<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::gateway::v1::ListProjectGatewaysRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::gateway::v1::ListProjectGatewaysResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the GetGateway RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_gateway<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::gateway::v1::GetGatewayRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::gateway::v1::GetGatewayResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Installs the gateway declarations from one published release's exact
    /// repository commit. The manifest is resolved by the service and is never
    /// supplied by a caller.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn install_release_gateways<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::gateway::v1::InstallReleaseGatewaysRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::gateway::v1::InstallReleaseGatewaysResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Creates one immutable runtime revision from the currently active
    /// published declaration. Runtime values and secret versions are selected
    /// explicitly; source declarations remain release owned.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn configure_gateway<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::gateway::v1::ConfigureGatewayRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::gateway::v1::ConfigureGatewayResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ListGatewayIngress RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_gateway_ingress<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::gateway::v1::ListGatewayIngressRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::gateway::v1::ListGatewayIngressResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the SetGatewayLifecycle RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn set_gateway_lifecycle<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::gateway::v1::SetGatewayLifecycleRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::gateway::v1::SetGatewayLifecycleResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Binds one immutable, declared gateway revision slot to exactly one
    /// same-project mailbox and creates its separately revocable publish grant.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn create_mailbox_binding<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::gateway::v1::CreateMailboxBindingRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::gateway::v1::CreateMailboxBindingResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Revokes an active gateway mailbox publish grant without deleting its
    /// immutable binding or redacted provenance.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn revoke_mailbox_binding_grant<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::gateway::v1::RevokeMailboxBindingGrantRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::gateway::v1::RevokeMailboxBindingGrantResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Lists value-free mailbox bindings and their grant lifecycle for one
    /// readable gateway revision.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_mailbox_bindings<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::gateway::v1::ListMailboxBindingsRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::gateway::v1::ListMailboxBindingsResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Lists value-free publication provenance for one readable gateway.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn list_mailbox_publications<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::gateway::v1::ListMailboxPublicationsRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::gateway::v1::ListMailboxPublicationsResponse,
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
pub trait GatewayServiceExt: GatewayService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: GatewayService> GatewayServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view_idempotent(
                GATEWAY_SERVICE_SERVICE_NAME,
                "ListProjectGateways",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::gateway::v1::__buffa::view::ListProjectGatewaysRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::gateway::v1::ListProjectGatewaysRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_project_gateways(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::gateway::v1::ListProjectGatewaysResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(GATEWAY_SERVICE_LIST_PROJECT_GATEWAYS_SPEC)
            .route_view_idempotent(
                GATEWAY_SERVICE_SERVICE_NAME,
                "GetGateway",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::gateway::v1::__buffa::view::GetGatewayRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::gateway::v1::GetGatewayRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_gateway(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::gateway::v1::GetGatewayResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(GATEWAY_SERVICE_GET_GATEWAY_SPEC)
            .route_view(
                GATEWAY_SERVICE_SERVICE_NAME,
                "InstallReleaseGateways",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::gateway::v1::__buffa::view::InstallReleaseGatewaysRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::gateway::v1::InstallReleaseGatewaysRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.install_release_gateways(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::gateway::v1::InstallReleaseGatewaysResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(GATEWAY_SERVICE_INSTALL_RELEASE_GATEWAYS_SPEC)
            .route_view(
                GATEWAY_SERVICE_SERVICE_NAME,
                "ConfigureGateway",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::gateway::v1::__buffa::view::ConfigureGatewayRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::gateway::v1::ConfigureGatewayRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.configure_gateway(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::gateway::v1::ConfigureGatewayResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(GATEWAY_SERVICE_CONFIGURE_GATEWAY_SPEC)
            .route_view_idempotent(
                GATEWAY_SERVICE_SERVICE_NAME,
                "ListGatewayIngress",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::gateway::v1::__buffa::view::ListGatewayIngressRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::gateway::v1::ListGatewayIngressRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_gateway_ingress(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::gateway::v1::ListGatewayIngressResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(GATEWAY_SERVICE_LIST_GATEWAY_INGRESS_SPEC)
            .route_view(
                GATEWAY_SERVICE_SERVICE_NAME,
                "SetGatewayLifecycle",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::gateway::v1::__buffa::view::SetGatewayLifecycleRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::gateway::v1::SetGatewayLifecycleRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.set_gateway_lifecycle(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::gateway::v1::SetGatewayLifecycleResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(GATEWAY_SERVICE_SET_GATEWAY_LIFECYCLE_SPEC)
            .route_view(
                GATEWAY_SERVICE_SERVICE_NAME,
                "CreateMailboxBinding",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::gateway::v1::__buffa::view::CreateMailboxBindingRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::gateway::v1::CreateMailboxBindingRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.create_mailbox_binding(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::gateway::v1::CreateMailboxBindingResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(GATEWAY_SERVICE_CREATE_MAILBOX_BINDING_SPEC)
            .route_view(
                GATEWAY_SERVICE_SERVICE_NAME,
                "RevokeMailboxBindingGrant",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::gateway::v1::__buffa::view::RevokeMailboxBindingGrantRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::gateway::v1::RevokeMailboxBindingGrantRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.revoke_mailbox_binding_grant(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::gateway::v1::RevokeMailboxBindingGrantResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(GATEWAY_SERVICE_REVOKE_MAILBOX_BINDING_GRANT_SPEC)
            .route_view_idempotent(
                GATEWAY_SERVICE_SERVICE_NAME,
                "ListMailboxBindings",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxBindingsRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::gateway::v1::ListMailboxBindingsRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_mailbox_bindings(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::gateway::v1::ListMailboxBindingsResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(GATEWAY_SERVICE_LIST_MAILBOX_BINDINGS_SPEC)
            .route_view_idempotent(
                GATEWAY_SERVICE_SERVICE_NAME,
                "ListMailboxPublications",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxPublicationsRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::gateway::v1::ListMailboxPublicationsRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.list_mailbox_publications(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::gateway::v1::ListMailboxPublicationsResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(GATEWAY_SERVICE_LIST_MAILBOX_PUBLICATIONS_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct GatewayServiceRegisterMarker;
impl<S: GatewayService> ::connectrpc::ServiceRegister<GatewayServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as GatewayServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `GatewayService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = GatewayServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct GatewayServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: GatewayService> GatewayServiceServer<T> {
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
impl<T> Clone for GatewayServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: GatewayService> ::connectrpc::Dispatcher for GatewayServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("hephaestus.gateway.v1.GatewayService/")?;
        match method {
            "ListProjectGateways" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(GATEWAY_SERVICE_LIST_PROJECT_GATEWAYS_SPEC),
                )
            }
            "GetGateway" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(GATEWAY_SERVICE_GET_GATEWAY_SPEC),
                )
            }
            "InstallReleaseGateways" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(GATEWAY_SERVICE_INSTALL_RELEASE_GATEWAYS_SPEC),
                )
            }
            "ConfigureGateway" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(GATEWAY_SERVICE_CONFIGURE_GATEWAY_SPEC),
                )
            }
            "ListGatewayIngress" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(GATEWAY_SERVICE_LIST_GATEWAY_INGRESS_SPEC),
                )
            }
            "SetGatewayLifecycle" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(GATEWAY_SERVICE_SET_GATEWAY_LIFECYCLE_SPEC),
                )
            }
            "CreateMailboxBinding" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(GATEWAY_SERVICE_CREATE_MAILBOX_BINDING_SPEC),
                )
            }
            "RevokeMailboxBindingGrant" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(GATEWAY_SERVICE_REVOKE_MAILBOX_BINDING_GRANT_SPEC),
                )
            }
            "ListMailboxBindings" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(GATEWAY_SERVICE_LIST_MAILBOX_BINDINGS_SPEC),
                )
            }
            "ListMailboxPublications" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(GATEWAY_SERVICE_LIST_MAILBOX_PUBLICATIONS_SPEC),
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
        let Some(method) = path.strip_prefix("hephaestus.gateway.v1.GatewayService/")
        else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "ListProjectGateways" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::gateway::v1::ListProjectGatewaysRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::gateway::v1::__buffa::view::ListProjectGatewaysRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::gateway::v1::ListProjectGatewaysRequest,
                    >::from_parts(&req, &body);
                    svc.list_project_gateways(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::gateway::v1::ListProjectGatewaysResponse,
                        >(format)
                })
            }
            "GetGateway" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::gateway::v1::GetGatewayRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::gateway::v1::__buffa::view::GetGatewayRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::gateway::v1::GetGatewayRequest,
                    >::from_parts(&req, &body);
                    svc.get_gateway(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::gateway::v1::GetGatewayResponse,
                        >(format)
                })
            }
            "InstallReleaseGateways" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::gateway::v1::InstallReleaseGatewaysRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::gateway::v1::__buffa::view::InstallReleaseGatewaysRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::gateway::v1::InstallReleaseGatewaysRequest,
                    >::from_parts(&req, &body);
                    svc.install_release_gateways(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::gateway::v1::InstallReleaseGatewaysResponse,
                        >(format)
                })
            }
            "ConfigureGateway" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::gateway::v1::ConfigureGatewayRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::gateway::v1::__buffa::view::ConfigureGatewayRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::gateway::v1::ConfigureGatewayRequest,
                    >::from_parts(&req, &body);
                    svc.configure_gateway(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::gateway::v1::ConfigureGatewayResponse,
                        >(format)
                })
            }
            "ListGatewayIngress" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::gateway::v1::ListGatewayIngressRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::gateway::v1::__buffa::view::ListGatewayIngressRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::gateway::v1::ListGatewayIngressRequest,
                    >::from_parts(&req, &body);
                    svc.list_gateway_ingress(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::gateway::v1::ListGatewayIngressResponse,
                        >(format)
                })
            }
            "SetGatewayLifecycle" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::gateway::v1::SetGatewayLifecycleRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::gateway::v1::__buffa::view::SetGatewayLifecycleRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::gateway::v1::SetGatewayLifecycleRequest,
                    >::from_parts(&req, &body);
                    svc.set_gateway_lifecycle(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::gateway::v1::SetGatewayLifecycleResponse,
                        >(format)
                })
            }
            "CreateMailboxBinding" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::gateway::v1::CreateMailboxBindingRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::gateway::v1::__buffa::view::CreateMailboxBindingRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::gateway::v1::CreateMailboxBindingRequest,
                    >::from_parts(&req, &body);
                    svc.create_mailbox_binding(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::gateway::v1::CreateMailboxBindingResponse,
                        >(format)
                })
            }
            "RevokeMailboxBindingGrant" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::gateway::v1::RevokeMailboxBindingGrantRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::gateway::v1::__buffa::view::RevokeMailboxBindingGrantRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::gateway::v1::RevokeMailboxBindingGrantRequest,
                    >::from_parts(&req, &body);
                    svc.revoke_mailbox_binding_grant(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::gateway::v1::RevokeMailboxBindingGrantResponse,
                        >(format)
                })
            }
            "ListMailboxBindings" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::gateway::v1::ListMailboxBindingsRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxBindingsRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::gateway::v1::ListMailboxBindingsRequest,
                    >::from_parts(&req, &body);
                    svc.list_mailbox_bindings(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::gateway::v1::ListMailboxBindingsResponse,
                        >(format)
                })
            }
            "ListMailboxPublications" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::gateway::v1::ListMailboxPublicationsRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxPublicationsRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::gateway::v1::ListMailboxPublicationsRequest,
                    >::from_parts(&req, &body);
                    svc.list_mailbox_publications(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::gateway::v1::ListMailboxPublicationsResponse,
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
        let Some(method) = path.strip_prefix("hephaestus.gateway.v1.GatewayService/")
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
        let Some(method) = path.strip_prefix("hephaestus.gateway.v1.GatewayService/")
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
        let Some(method) = path.strip_prefix("hephaestus.gateway.v1.GatewayService/")
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
/// let client = GatewayServiceClient::new(conn, config);
/// let response = client.list_project_gateways(request).await?;
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
/// let client = GatewayServiceClient::new(http, config);
/// let response = client.list_project_gateways(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.list_project_gateways(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.list_project_gateways(request).await?.into_owned();
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
pub struct GatewayServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> GatewayServiceClient<T>
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
    /// Call the ListProjectGateways RPC. Sends a request to /hephaestus.gateway.v1.GatewayService/ListProjectGateways.
    pub async fn list_project_gateways(
        &self,
        request: crate::messages::hephaestus::gateway::v1::ListProjectGatewaysRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::ListProjectGatewaysResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_project_gateways_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListProjectGateways RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_project_gateways_with_options(
        &self,
        request: crate::messages::hephaestus::gateway::v1::ListProjectGatewaysRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::ListProjectGatewaysResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                GATEWAY_SERVICE_SERVICE_NAME,
                "ListProjectGateways",
                request,
                options,
            )
            .await
    }
    /// Call the GetGateway RPC. Sends a request to /hephaestus.gateway.v1.GatewayService/GetGateway.
    pub async fn get_gateway(
        &self,
        request: crate::messages::hephaestus::gateway::v1::GetGatewayRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::GetGatewayResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_gateway_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the GetGateway RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_gateway_with_options(
        &self,
        request: crate::messages::hephaestus::gateway::v1::GetGatewayRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::GetGatewayResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                GATEWAY_SERVICE_SERVICE_NAME,
                "GetGateway",
                request,
                options,
            )
            .await
    }
    /// Call the InstallReleaseGateways RPC. Sends a request to /hephaestus.gateway.v1.GatewayService/InstallReleaseGateways.
    pub async fn install_release_gateways(
        &self,
        request: crate::messages::hephaestus::gateway::v1::InstallReleaseGatewaysRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::InstallReleaseGatewaysResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.install_release_gateways_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the InstallReleaseGateways RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn install_release_gateways_with_options(
        &self,
        request: crate::messages::hephaestus::gateway::v1::InstallReleaseGatewaysRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::InstallReleaseGatewaysResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                GATEWAY_SERVICE_SERVICE_NAME,
                "InstallReleaseGateways",
                request,
                options,
            )
            .await
    }
    /// Call the ConfigureGateway RPC. Sends a request to /hephaestus.gateway.v1.GatewayService/ConfigureGateway.
    pub async fn configure_gateway(
        &self,
        request: crate::messages::hephaestus::gateway::v1::ConfigureGatewayRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::ConfigureGatewayResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.configure_gateway_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ConfigureGateway RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn configure_gateway_with_options(
        &self,
        request: crate::messages::hephaestus::gateway::v1::ConfigureGatewayRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::ConfigureGatewayResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                GATEWAY_SERVICE_SERVICE_NAME,
                "ConfigureGateway",
                request,
                options,
            )
            .await
    }
    /// Call the ListGatewayIngress RPC. Sends a request to /hephaestus.gateway.v1.GatewayService/ListGatewayIngress.
    pub async fn list_gateway_ingress(
        &self,
        request: crate::messages::hephaestus::gateway::v1::ListGatewayIngressRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::ListGatewayIngressResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_gateway_ingress_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListGatewayIngress RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_gateway_ingress_with_options(
        &self,
        request: crate::messages::hephaestus::gateway::v1::ListGatewayIngressRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::ListGatewayIngressResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                GATEWAY_SERVICE_SERVICE_NAME,
                "ListGatewayIngress",
                request,
                options,
            )
            .await
    }
    /// Call the SetGatewayLifecycle RPC. Sends a request to /hephaestus.gateway.v1.GatewayService/SetGatewayLifecycle.
    pub async fn set_gateway_lifecycle(
        &self,
        request: crate::messages::hephaestus::gateway::v1::SetGatewayLifecycleRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::SetGatewayLifecycleResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.set_gateway_lifecycle_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the SetGatewayLifecycle RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn set_gateway_lifecycle_with_options(
        &self,
        request: crate::messages::hephaestus::gateway::v1::SetGatewayLifecycleRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::SetGatewayLifecycleResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                GATEWAY_SERVICE_SERVICE_NAME,
                "SetGatewayLifecycle",
                request,
                options,
            )
            .await
    }
    /// Call the CreateMailboxBinding RPC. Sends a request to /hephaestus.gateway.v1.GatewayService/CreateMailboxBinding.
    pub async fn create_mailbox_binding(
        &self,
        request: crate::messages::hephaestus::gateway::v1::CreateMailboxBindingRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::CreateMailboxBindingResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.create_mailbox_binding_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the CreateMailboxBinding RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn create_mailbox_binding_with_options(
        &self,
        request: crate::messages::hephaestus::gateway::v1::CreateMailboxBindingRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::CreateMailboxBindingResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                GATEWAY_SERVICE_SERVICE_NAME,
                "CreateMailboxBinding",
                request,
                options,
            )
            .await
    }
    /// Call the RevokeMailboxBindingGrant RPC. Sends a request to /hephaestus.gateway.v1.GatewayService/RevokeMailboxBindingGrant.
    pub async fn revoke_mailbox_binding_grant(
        &self,
        request: crate::messages::hephaestus::gateway::v1::RevokeMailboxBindingGrantRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::RevokeMailboxBindingGrantResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.revoke_mailbox_binding_grant_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the RevokeMailboxBindingGrant RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn revoke_mailbox_binding_grant_with_options(
        &self,
        request: crate::messages::hephaestus::gateway::v1::RevokeMailboxBindingGrantRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::RevokeMailboxBindingGrantResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                GATEWAY_SERVICE_SERVICE_NAME,
                "RevokeMailboxBindingGrant",
                request,
                options,
            )
            .await
    }
    /// Call the ListMailboxBindings RPC. Sends a request to /hephaestus.gateway.v1.GatewayService/ListMailboxBindings.
    pub async fn list_mailbox_bindings(
        &self,
        request: crate::messages::hephaestus::gateway::v1::ListMailboxBindingsRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxBindingsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_mailbox_bindings_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListMailboxBindings RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_mailbox_bindings_with_options(
        &self,
        request: crate::messages::hephaestus::gateway::v1::ListMailboxBindingsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxBindingsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                GATEWAY_SERVICE_SERVICE_NAME,
                "ListMailboxBindings",
                request,
                options,
            )
            .await
    }
    /// Call the ListMailboxPublications RPC. Sends a request to /hephaestus.gateway.v1.GatewayService/ListMailboxPublications.
    pub async fn list_mailbox_publications(
        &self,
        request: crate::messages::hephaestus::gateway::v1::ListMailboxPublicationsRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxPublicationsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.list_mailbox_publications_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ListMailboxPublications RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn list_mailbox_publications_with_options(
        &self,
        request: crate::messages::hephaestus::gateway::v1::ListMailboxPublicationsRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::gateway::v1::__buffa::view::ListMailboxPublicationsResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                GATEWAY_SERVICE_SERVICE_NAME,
                "ListMailboxPublications",
                request,
                options,
            )
            .await
    }
}
