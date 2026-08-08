///Shorthand for `OwnedView<GetInstanceRequestView<'static>>`.
pub type OwnedGetInstanceRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::GetInstanceRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<GetInstanceResponseView<'static>>`.
pub type OwnedGetInstanceResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::GetInstanceResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ImportAgentRequestView<'static>>`.
pub type OwnedImportAgentRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::ImportAgentRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ImportAgentResponseView<'static>>`.
pub type OwnedImportAgentResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::ImportAgentResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreateAttachmentRequestView<'static>>`.
pub type OwnedCreateAttachmentRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::CreateAttachmentRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreateAttachmentResponseView<'static>>`.
pub type OwnedCreateAttachmentResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::CreateAttachmentResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<SetAttachmentEnabledRequestView<'static>>`.
pub type OwnedSetAttachmentEnabledRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::SetAttachmentEnabledRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<SetAttachmentEnabledResponseView<'static>>`.
pub type OwnedSetAttachmentEnabledResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::SetAttachmentEnabledResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RemoveAttachmentRequestView<'static>>`.
pub type OwnedRemoveAttachmentRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::RemoveAttachmentRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RemoveAttachmentResponseView<'static>>`.
pub type OwnedRemoveAttachmentResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::RemoveAttachmentResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ReviseInstanceRequestView<'static>>`.
pub type OwnedReviseInstanceRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::ReviseInstanceRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ReviseInstanceResponseView<'static>>`.
pub type OwnedReviseInstanceResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::ReviseInstanceResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreateUpdateRequestView<'static>>`.
pub type OwnedCreateUpdateRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::CreateUpdateRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<CreateUpdateResponseView<'static>>`.
pub type OwnedCreateUpdateResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::CreateUpdateResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RecoverUpdateRequestView<'static>>`.
pub type OwnedRecoverUpdateRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::RecoverUpdateRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<RecoverUpdateResponseView<'static>>`.
pub type OwnedRecoverUpdateResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::RecoverUpdateResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<BindSecretRequestView<'static>>`.
pub type OwnedBindSecretRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::BindSecretRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<BindSecretResponseView<'static>>`.
pub type OwnedBindSecretResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::BindSecretResponseView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ReviseCapabilitiesRequestView<'static>>`.
pub type OwnedReviseCapabilitiesRequestView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::ReviseCapabilitiesRequestView<
        'static,
    >,
>;
///Shorthand for `OwnedView<ReviseCapabilitiesResponseView<'static>>`.
pub type OwnedReviseCapabilitiesResponseView = ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::ReviseCapabilitiesResponseView<
        'static,
    >,
>;
impl ::connectrpc::Encodable<
    crate::messages::hephaestus::instance::v1::GetInstanceResponse,
>
for crate::messages::hephaestus::instance::v1::__buffa::view::GetInstanceResponseView<
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
    crate::messages::hephaestus::instance::v1::GetInstanceResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::GetInstanceResponseView<
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
    crate::messages::hephaestus::instance::v1::ImportAgentResponse,
>
for crate::messages::hephaestus::instance::v1::__buffa::view::ImportAgentResponseView<
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
    crate::messages::hephaestus::instance::v1::ImportAgentResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::ImportAgentResponseView<
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
    crate::messages::hephaestus::instance::v1::CreateAttachmentResponse,
>
for crate::messages::hephaestus::instance::v1::__buffa::view::CreateAttachmentResponseView<
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
    crate::messages::hephaestus::instance::v1::CreateAttachmentResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::CreateAttachmentResponseView<
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
    crate::messages::hephaestus::instance::v1::SetAttachmentEnabledResponse,
>
for crate::messages::hephaestus::instance::v1::__buffa::view::SetAttachmentEnabledResponseView<
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
    crate::messages::hephaestus::instance::v1::SetAttachmentEnabledResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::SetAttachmentEnabledResponseView<
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
    crate::messages::hephaestus::instance::v1::RemoveAttachmentResponse,
>
for crate::messages::hephaestus::instance::v1::__buffa::view::RemoveAttachmentResponseView<
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
    crate::messages::hephaestus::instance::v1::RemoveAttachmentResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::RemoveAttachmentResponseView<
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
    crate::messages::hephaestus::instance::v1::ReviseInstanceResponse,
>
for crate::messages::hephaestus::instance::v1::__buffa::view::ReviseInstanceResponseView<
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
    crate::messages::hephaestus::instance::v1::ReviseInstanceResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::ReviseInstanceResponseView<
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
    crate::messages::hephaestus::instance::v1::CreateUpdateResponse,
>
for crate::messages::hephaestus::instance::v1::__buffa::view::CreateUpdateResponseView<
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
    crate::messages::hephaestus::instance::v1::CreateUpdateResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::CreateUpdateResponseView<
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
    crate::messages::hephaestus::instance::v1::RecoverUpdateResponse,
>
for crate::messages::hephaestus::instance::v1::__buffa::view::RecoverUpdateResponseView<
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
    crate::messages::hephaestus::instance::v1::RecoverUpdateResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::RecoverUpdateResponseView<
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
    crate::messages::hephaestus::instance::v1::BindSecretResponse,
>
for crate::messages::hephaestus::instance::v1::__buffa::view::BindSecretResponseView<
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
    crate::messages::hephaestus::instance::v1::BindSecretResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::BindSecretResponseView<
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
    crate::messages::hephaestus::instance::v1::ReviseCapabilitiesResponse,
>
for crate::messages::hephaestus::instance::v1::__buffa::view::ReviseCapabilitiesResponseView<
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
    crate::messages::hephaestus::instance::v1::ReviseCapabilitiesResponse,
>
for ::buffa::view::OwnedView<
    crate::messages::hephaestus::instance::v1::__buffa::view::ReviseCapabilitiesResponseView<
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
pub const AGENT_INSTANCE_SERVICE_SERVICE_NAME: &str = "hephaestus.instance.v1.AgentInstanceService";
/// Static [`Spec`](::connectrpc::Spec) for the server-side `GetInstance` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const AGENT_INSTANCE_SERVICE_GET_INSTANCE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.instance.v1.AgentInstanceService/GetInstance",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::NoSideEffects);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ImportAgent` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const AGENT_INSTANCE_SERVICE_IMPORT_AGENT_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.instance.v1.AgentInstanceService/ImportAgent",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `CreateAttachment` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const AGENT_INSTANCE_SERVICE_CREATE_ATTACHMENT_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.instance.v1.AgentInstanceService/CreateAttachment",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `SetAttachmentEnabled` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const AGENT_INSTANCE_SERVICE_SET_ATTACHMENT_ENABLED_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.instance.v1.AgentInstanceService/SetAttachmentEnabled",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `RemoveAttachment` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const AGENT_INSTANCE_SERVICE_REMOVE_ATTACHMENT_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.instance.v1.AgentInstanceService/RemoveAttachment",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ReviseInstance` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const AGENT_INSTANCE_SERVICE_REVISE_INSTANCE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.instance.v1.AgentInstanceService/ReviseInstance",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `CreateUpdate` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const AGENT_INSTANCE_SERVICE_CREATE_UPDATE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.instance.v1.AgentInstanceService/CreateUpdate",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `RecoverUpdate` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const AGENT_INSTANCE_SERVICE_RECOVER_UPDATE_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.instance.v1.AgentInstanceService/RecoverUpdate",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `BindSecret` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const AGENT_INSTANCE_SERVICE_BIND_SECRET_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.instance.v1.AgentInstanceService/BindSecret",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Static [`Spec`](::connectrpc::Spec) for the server-side `ReviseCapabilities` RPC.
///
/// The dispatcher surfaces this on
/// [`RequestContext::spec`](::connectrpc::RequestContext::spec).
pub const AGENT_INSTANCE_SERVICE_REVISE_CAPABILITIES_SPEC: ::connectrpc::Spec = ::connectrpc::Spec::server(
        "/hephaestus.instance.v1.AgentInstanceService/ReviseCapabilities",
        ::connectrpc::StreamType::Unary,
    )
    .with_idempotency_level(::connectrpc::IdempotencyLevel::Unknown);
/// Server trait for AgentInstanceService.
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
pub trait AgentInstanceService: Send + Sync + 'static {
    /// Handle the GetInstance RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn get_instance<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::instance::v1::GetInstanceRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::instance::v1::GetInstanceResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ImportAgent RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn import_agent<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::instance::v1::ImportAgentRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::instance::v1::ImportAgentResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the CreateAttachment RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn create_attachment<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::instance::v1::CreateAttachmentRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::instance::v1::CreateAttachmentResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the SetAttachmentEnabled RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn set_attachment_enabled<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::instance::v1::SetAttachmentEnabledRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::instance::v1::SetAttachmentEnabledResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the RemoveAttachment RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn remove_attachment<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::instance::v1::RemoveAttachmentRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::instance::v1::RemoveAttachmentResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ReviseInstance RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn revise_instance<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::instance::v1::ReviseInstanceRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::instance::v1::ReviseInstanceResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the CreateUpdate RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn create_update<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::instance::v1::CreateUpdateRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::instance::v1::CreateUpdateResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the RecoverUpdate RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn recover_update<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::instance::v1::RecoverUpdateRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::instance::v1::RecoverUpdateResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the BindSecret RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn bind_secret<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::instance::v1::BindSecretRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::instance::v1::BindSecretResponse,
            > + Send + use<'a, Self>,
        >,
    > + Send;
    /// Handle the ReviseCapabilities RPC.
    ///
    /// `'a` lets the response body borrow from `&self` (e.g. server-resident state).
    ///
    /// `request` is borrowed from the request body and is valid for the
    /// duration of the call; message fields are read directly on it
    /// (zero-copy). The response cannot borrow from `request` — use
    /// `.to_owned_message()` (or copy the specific fields) for anything
    /// returned, stored, or moved into `tokio::spawn`.
    fn revise_capabilities<'a>(
        &'a self,
        ctx: ::connectrpc::RequestContext,
        request: ::connectrpc::ServiceRequest<
            '_,
            crate::messages::hephaestus::instance::v1::ReviseCapabilitiesRequest,
        >,
    ) -> impl ::std::future::Future<
        Output = ::connectrpc::ServiceResult<
            impl ::connectrpc::Encodable<
                crate::messages::hephaestus::instance::v1::ReviseCapabilitiesResponse,
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
pub trait AgentInstanceServiceExt: AgentInstanceService {
    /// Register this service implementation with a Router.
    ///
    /// Takes ownership of the `Arc<Self>` and returns a new Router with
    /// this service's methods registered.
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router;
}
impl<S: AgentInstanceService> AgentInstanceServiceExt for S {
    fn register(
        self: ::std::sync::Arc<Self>,
        router: ::connectrpc::Router,
    ) -> ::connectrpc::Router {
        router
            .route_view_idempotent(
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "GetInstance",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::instance::v1::__buffa::view::GetInstanceRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::instance::v1::GetInstanceRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.get_instance(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::instance::v1::GetInstanceResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(AGENT_INSTANCE_SERVICE_GET_INSTANCE_SPEC)
            .route_view(
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "ImportAgent",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::instance::v1::__buffa::view::ImportAgentRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::instance::v1::ImportAgentRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.import_agent(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::instance::v1::ImportAgentResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(AGENT_INSTANCE_SERVICE_IMPORT_AGENT_SPEC)
            .route_view(
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "CreateAttachment",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::instance::v1::__buffa::view::CreateAttachmentRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::instance::v1::CreateAttachmentRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.create_attachment(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::instance::v1::CreateAttachmentResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(AGENT_INSTANCE_SERVICE_CREATE_ATTACHMENT_SPEC)
            .route_view(
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "SetAttachmentEnabled",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::instance::v1::__buffa::view::SetAttachmentEnabledRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::instance::v1::SetAttachmentEnabledRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.set_attachment_enabled(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::instance::v1::SetAttachmentEnabledResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(AGENT_INSTANCE_SERVICE_SET_ATTACHMENT_ENABLED_SPEC)
            .route_view(
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "RemoveAttachment",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::instance::v1::__buffa::view::RemoveAttachmentRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::instance::v1::RemoveAttachmentRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.remove_attachment(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::instance::v1::RemoveAttachmentResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(AGENT_INSTANCE_SERVICE_REMOVE_ATTACHMENT_SPEC)
            .route_view(
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "ReviseInstance",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::instance::v1::__buffa::view::ReviseInstanceRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::instance::v1::ReviseInstanceRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.revise_instance(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::instance::v1::ReviseInstanceResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(AGENT_INSTANCE_SERVICE_REVISE_INSTANCE_SPEC)
            .route_view(
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "CreateUpdate",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::instance::v1::__buffa::view::CreateUpdateRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::instance::v1::CreateUpdateRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.create_update(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::instance::v1::CreateUpdateResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(AGENT_INSTANCE_SERVICE_CREATE_UPDATE_SPEC)
            .route_view(
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "RecoverUpdate",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::instance::v1::__buffa::view::RecoverUpdateRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::instance::v1::RecoverUpdateRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.recover_update(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::instance::v1::RecoverUpdateResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(AGENT_INSTANCE_SERVICE_RECOVER_UPDATE_SPEC)
            .route_view(
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "BindSecret",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::instance::v1::__buffa::view::BindSecretRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::instance::v1::BindSecretRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.bind_secret(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::instance::v1::BindSecretResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(AGENT_INSTANCE_SERVICE_BIND_SECRET_SPEC)
            .route_view(
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "ReviseCapabilities",
                {
                    let svc = ::std::sync::Arc::clone(&self);
                    ::connectrpc::view_handler_fn(move |
                        ctx,
                        req: ::buffa::view::OwnedView<
                            crate::messages::hephaestus::instance::v1::__buffa::view::ReviseCapabilitiesRequestView<
                                'static,
                            >,
                        >,
                        format|
                    {
                        let svc = ::std::sync::Arc::clone(&svc);
                        async move {
                            let sreq = ::connectrpc::ServiceRequest::<
                                crate::messages::hephaestus::instance::v1::ReviseCapabilitiesRequest,
                            >::from_parts(req.reborrow(), req.bytes());
                            svc.revise_capabilities(ctx, sreq)
                                .await?
                                .encode::<
                                    crate::messages::hephaestus::instance::v1::ReviseCapabilitiesResponse,
                                >(format)
                        }
                    })
                },
            )
            .with_spec(AGENT_INSTANCE_SERVICE_REVISE_CAPABILITIES_SPEC)
    }
}
/// Type-inference marker used by [`Router::add_service`](::connectrpc::Router::add_service).
#[doc(hidden)]
pub struct AgentInstanceServiceRegisterMarker;
impl<
    S: AgentInstanceService,
> ::connectrpc::ServiceRegister<AgentInstanceServiceRegisterMarker>
for ::std::sync::Arc<S> {
    fn register_service(self, router: ::connectrpc::Router) -> ::connectrpc::Router {
        <S as AgentInstanceServiceExt>::register(self, router)
    }
}
/// Monomorphic dispatcher for `AgentInstanceService`.
///
/// Unlike `.register(Router)` which type-erases each method into an `Arc<dyn ErasedHandler>` stored in a `HashMap`, this struct dispatches via a compile-time `match` on method name: no vtable, no hash lookup.
///
/// # Example
///
/// ```rust,ignore
/// use connectrpc::ConnectRpcService;
///
/// let server = AgentInstanceServiceServer::new(MyImpl);
/// let service = ConnectRpcService::new(server);
/// // hand `service` to axum/hyper as a fallback_service
/// ```
pub struct AgentInstanceServiceServer<T> {
    inner: ::std::sync::Arc<T>,
}
impl<T: AgentInstanceService> AgentInstanceServiceServer<T> {
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
impl<T> Clone for AgentInstanceServiceServer<T> {
    fn clone(&self) -> Self {
        Self {
            inner: ::std::sync::Arc::clone(&self.inner),
        }
    }
}
impl<T: AgentInstanceService> ::connectrpc::Dispatcher
for AgentInstanceServiceServer<T> {
    #[inline]
    fn lookup(
        &self,
        path: &str,
    ) -> Option<::connectrpc::dispatcher::codegen::MethodDescriptor> {
        let method = path.strip_prefix("hephaestus.instance.v1.AgentInstanceService/")?;
        match method {
            "GetInstance" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(true)
                        .with_spec(AGENT_INSTANCE_SERVICE_GET_INSTANCE_SPEC),
                )
            }
            "ImportAgent" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(AGENT_INSTANCE_SERVICE_IMPORT_AGENT_SPEC),
                )
            }
            "CreateAttachment" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(AGENT_INSTANCE_SERVICE_CREATE_ATTACHMENT_SPEC),
                )
            }
            "SetAttachmentEnabled" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(AGENT_INSTANCE_SERVICE_SET_ATTACHMENT_ENABLED_SPEC),
                )
            }
            "RemoveAttachment" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(AGENT_INSTANCE_SERVICE_REMOVE_ATTACHMENT_SPEC),
                )
            }
            "ReviseInstance" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(AGENT_INSTANCE_SERVICE_REVISE_INSTANCE_SPEC),
                )
            }
            "CreateUpdate" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(AGENT_INSTANCE_SERVICE_CREATE_UPDATE_SPEC),
                )
            }
            "RecoverUpdate" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(AGENT_INSTANCE_SERVICE_RECOVER_UPDATE_SPEC),
                )
            }
            "BindSecret" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(AGENT_INSTANCE_SERVICE_BIND_SECRET_SPEC),
                )
            }
            "ReviseCapabilities" => {
                Some(
                    ::connectrpc::dispatcher::codegen::MethodDescriptor::unary(false)
                        .with_spec(AGENT_INSTANCE_SERVICE_REVISE_CAPABILITIES_SPEC),
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
            .strip_prefix("hephaestus.instance.v1.AgentInstanceService/") else {
            return ::connectrpc::dispatcher::codegen::unimplemented_unary(path);
        };
        let _ = (&ctx, &request, &format);
        match method {
            "GetInstance" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::instance::v1::GetInstanceRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::instance::v1::__buffa::view::GetInstanceRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::instance::v1::GetInstanceRequest,
                    >::from_parts(&req, &body);
                    svc.get_instance(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::instance::v1::GetInstanceResponse,
                        >(format)
                })
            }
            "ImportAgent" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::instance::v1::ImportAgentRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::instance::v1::__buffa::view::ImportAgentRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::instance::v1::ImportAgentRequest,
                    >::from_parts(&req, &body);
                    svc.import_agent(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::instance::v1::ImportAgentResponse,
                        >(format)
                })
            }
            "CreateAttachment" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::instance::v1::CreateAttachmentRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::instance::v1::__buffa::view::CreateAttachmentRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::instance::v1::CreateAttachmentRequest,
                    >::from_parts(&req, &body);
                    svc.create_attachment(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::instance::v1::CreateAttachmentResponse,
                        >(format)
                })
            }
            "SetAttachmentEnabled" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::instance::v1::SetAttachmentEnabledRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::instance::v1::__buffa::view::SetAttachmentEnabledRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::instance::v1::SetAttachmentEnabledRequest,
                    >::from_parts(&req, &body);
                    svc.set_attachment_enabled(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::instance::v1::SetAttachmentEnabledResponse,
                        >(format)
                })
            }
            "RemoveAttachment" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::instance::v1::RemoveAttachmentRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::instance::v1::__buffa::view::RemoveAttachmentRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::instance::v1::RemoveAttachmentRequest,
                    >::from_parts(&req, &body);
                    svc.remove_attachment(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::instance::v1::RemoveAttachmentResponse,
                        >(format)
                })
            }
            "ReviseInstance" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::instance::v1::ReviseInstanceRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::instance::v1::__buffa::view::ReviseInstanceRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::instance::v1::ReviseInstanceRequest,
                    >::from_parts(&req, &body);
                    svc.revise_instance(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::instance::v1::ReviseInstanceResponse,
                        >(format)
                })
            }
            "CreateUpdate" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::instance::v1::CreateUpdateRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::instance::v1::__buffa::view::CreateUpdateRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::instance::v1::CreateUpdateRequest,
                    >::from_parts(&req, &body);
                    svc.create_update(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::instance::v1::CreateUpdateResponse,
                        >(format)
                })
            }
            "RecoverUpdate" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::instance::v1::RecoverUpdateRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::instance::v1::__buffa::view::RecoverUpdateRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::instance::v1::RecoverUpdateRequest,
                    >::from_parts(&req, &body);
                    svc.recover_update(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::instance::v1::RecoverUpdateResponse,
                        >(format)
                })
            }
            "BindSecret" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::instance::v1::BindSecretRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::instance::v1::__buffa::view::BindSecretRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::instance::v1::BindSecretRequest,
                    >::from_parts(&req, &body);
                    svc.bind_secret(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::instance::v1::BindSecretResponse,
                        >(format)
                })
            }
            "ReviseCapabilities" => {
                let svc = ::std::sync::Arc::clone(&self.inner);
                Box::pin(async move {
                    let body = ::connectrpc::dispatcher::codegen::request_proto_bytes::<
                        crate::messages::hephaestus::instance::v1::ReviseCapabilitiesRequest,
                    >(request.encoded()?, format)?;
                    let req: crate::messages::hephaestus::instance::v1::__buffa::view::ReviseCapabilitiesRequestView<
                        '_,
                    > = ::connectrpc::dispatcher::codegen::decode_borrowed_request_view(
                        &body,
                    )?;
                    let req = ::connectrpc::ServiceRequest::<
                        crate::messages::hephaestus::instance::v1::ReviseCapabilitiesRequest,
                    >::from_parts(&req, &body);
                    svc.revise_capabilities(ctx, req)
                        .await?
                        .encode::<
                            crate::messages::hephaestus::instance::v1::ReviseCapabilitiesResponse,
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
            .strip_prefix("hephaestus.instance.v1.AgentInstanceService/") else {
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
            .strip_prefix("hephaestus.instance.v1.AgentInstanceService/") else {
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
            .strip_prefix("hephaestus.instance.v1.AgentInstanceService/") else {
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
/// let client = AgentInstanceServiceClient::new(conn, config);
/// let response = client.get_instance(request).await?;
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
/// let client = AgentInstanceServiceClient::new(http, config);
/// let response = client.get_instance(request).await?;
/// ```
///
/// # Working with the response
///
/// Unary calls return [`UnaryResponse<OwnedView<FooView>>`](::connectrpc::client::UnaryResponse).
/// [`view()`](::connectrpc::client::UnaryResponse::view) borrows the response
/// message, so field access is zero-copy:
///
/// ```rust,ignore
/// let resp = client.get_instance(request).await?;
/// let name: &str = resp.view().name;  // borrow into the response buffer
/// ```
///
/// If you need the owned struct (e.g. to store or pass by value), use
/// [`into_owned()`](::connectrpc::client::UnaryResponse::into_owned):
///
/// ```rust,ignore
/// let owned = client.get_instance(request).await?.into_owned();
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
pub struct AgentInstanceServiceClient<T> {
    transport: T,
    config: ::connectrpc::client::ClientConfig,
}
#[cfg(feature = "client")]
impl<T> AgentInstanceServiceClient<T>
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
    /// Call the GetInstance RPC. Sends a request to /hephaestus.instance.v1.AgentInstanceService/GetInstance.
    pub async fn get_instance(
        &self,
        request: crate::messages::hephaestus::instance::v1::GetInstanceRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::GetInstanceResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.get_instance_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the GetInstance RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn get_instance_with_options(
        &self,
        request: crate::messages::hephaestus::instance::v1::GetInstanceRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::GetInstanceResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "GetInstance",
                request,
                options,
            )
            .await
    }
    /// Call the ImportAgent RPC. Sends a request to /hephaestus.instance.v1.AgentInstanceService/ImportAgent.
    pub async fn import_agent(
        &self,
        request: crate::messages::hephaestus::instance::v1::ImportAgentRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::ImportAgentResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.import_agent_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ImportAgent RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn import_agent_with_options(
        &self,
        request: crate::messages::hephaestus::instance::v1::ImportAgentRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::ImportAgentResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "ImportAgent",
                request,
                options,
            )
            .await
    }
    /// Call the CreateAttachment RPC. Sends a request to /hephaestus.instance.v1.AgentInstanceService/CreateAttachment.
    pub async fn create_attachment(
        &self,
        request: crate::messages::hephaestus::instance::v1::CreateAttachmentRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::CreateAttachmentResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.create_attachment_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the CreateAttachment RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn create_attachment_with_options(
        &self,
        request: crate::messages::hephaestus::instance::v1::CreateAttachmentRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::CreateAttachmentResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "CreateAttachment",
                request,
                options,
            )
            .await
    }
    /// Call the SetAttachmentEnabled RPC. Sends a request to /hephaestus.instance.v1.AgentInstanceService/SetAttachmentEnabled.
    pub async fn set_attachment_enabled(
        &self,
        request: crate::messages::hephaestus::instance::v1::SetAttachmentEnabledRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::SetAttachmentEnabledResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.set_attachment_enabled_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the SetAttachmentEnabled RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn set_attachment_enabled_with_options(
        &self,
        request: crate::messages::hephaestus::instance::v1::SetAttachmentEnabledRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::SetAttachmentEnabledResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "SetAttachmentEnabled",
                request,
                options,
            )
            .await
    }
    /// Call the RemoveAttachment RPC. Sends a request to /hephaestus.instance.v1.AgentInstanceService/RemoveAttachment.
    pub async fn remove_attachment(
        &self,
        request: crate::messages::hephaestus::instance::v1::RemoveAttachmentRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::RemoveAttachmentResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.remove_attachment_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the RemoveAttachment RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn remove_attachment_with_options(
        &self,
        request: crate::messages::hephaestus::instance::v1::RemoveAttachmentRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::RemoveAttachmentResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "RemoveAttachment",
                request,
                options,
            )
            .await
    }
    /// Call the ReviseInstance RPC. Sends a request to /hephaestus.instance.v1.AgentInstanceService/ReviseInstance.
    pub async fn revise_instance(
        &self,
        request: crate::messages::hephaestus::instance::v1::ReviseInstanceRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::ReviseInstanceResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.revise_instance_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ReviseInstance RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn revise_instance_with_options(
        &self,
        request: crate::messages::hephaestus::instance::v1::ReviseInstanceRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::ReviseInstanceResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "ReviseInstance",
                request,
                options,
            )
            .await
    }
    /// Call the CreateUpdate RPC. Sends a request to /hephaestus.instance.v1.AgentInstanceService/CreateUpdate.
    pub async fn create_update(
        &self,
        request: crate::messages::hephaestus::instance::v1::CreateUpdateRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::CreateUpdateResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.create_update_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the CreateUpdate RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn create_update_with_options(
        &self,
        request: crate::messages::hephaestus::instance::v1::CreateUpdateRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::CreateUpdateResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "CreateUpdate",
                request,
                options,
            )
            .await
    }
    /// Call the RecoverUpdate RPC. Sends a request to /hephaestus.instance.v1.AgentInstanceService/RecoverUpdate.
    pub async fn recover_update(
        &self,
        request: crate::messages::hephaestus::instance::v1::RecoverUpdateRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::RecoverUpdateResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.recover_update_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the RecoverUpdate RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn recover_update_with_options(
        &self,
        request: crate::messages::hephaestus::instance::v1::RecoverUpdateRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::RecoverUpdateResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "RecoverUpdate",
                request,
                options,
            )
            .await
    }
    /// Call the BindSecret RPC. Sends a request to /hephaestus.instance.v1.AgentInstanceService/BindSecret.
    pub async fn bind_secret(
        &self,
        request: crate::messages::hephaestus::instance::v1::BindSecretRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::BindSecretResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.bind_secret_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the BindSecret RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn bind_secret_with_options(
        &self,
        request: crate::messages::hephaestus::instance::v1::BindSecretRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::BindSecretResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "BindSecret",
                request,
                options,
            )
            .await
    }
    /// Call the ReviseCapabilities RPC. Sends a request to /hephaestus.instance.v1.AgentInstanceService/ReviseCapabilities.
    pub async fn revise_capabilities(
        &self,
        request: crate::messages::hephaestus::instance::v1::ReviseCapabilitiesRequest,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::ReviseCapabilitiesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        self.revise_capabilities_with_options(
                request,
                ::connectrpc::client::CallOptions::default(),
            )
            .await
    }
    /// Call the ReviseCapabilities RPC with explicit per-call options. Options override [`ClientConfig`](::connectrpc::client::ClientConfig) defaults.
    pub async fn revise_capabilities_with_options(
        &self,
        request: crate::messages::hephaestus::instance::v1::ReviseCapabilitiesRequest,
        options: ::connectrpc::client::CallOptions,
    ) -> Result<
        ::connectrpc::client::UnaryResponse<
            ::buffa::view::OwnedView<
                crate::messages::hephaestus::instance::v1::__buffa::view::ReviseCapabilitiesResponseView<
                    'static,
                >,
            >,
        >,
        ::connectrpc::ConnectError,
    > {
        ::connectrpc::client::call_unary(
                &self.transport,
                &self.config,
                AGENT_INSTANCE_SERVICE_SERVICE_NAME,
                "ReviseCapabilities",
                request,
                options,
            )
            .await
    }
}
