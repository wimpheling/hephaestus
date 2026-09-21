defmodule Hephaestus.Identity.V1.ResolveIdentityRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.identity.v1.ResolveIdentityRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:issuer, 2, type: :string)
  field(:subject, 3, type: :string)
  field(:display_name, 4, type: :string, json_name: "displayName")
  field(:email, 5, type: :string)
  field(:email_verified, 6, type: :bool, json_name: "emailVerified")
end

defmodule Hephaestus.Identity.V1.ResolveIdentityResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.identity.v1.ResolveIdentityResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:user_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "userId")
  field(:display_name, 2, type: :string, json_name: "displayName")
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Identity.V1.CreateBrowserSessionRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.identity.v1.CreateBrowserSessionRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:issuer, 2, type: :string)
  field(:subject, 3, type: :string)
  field(:sid, 4, type: :bytes, deprecated: false)
end

defmodule Hephaestus.Identity.V1.CreateBrowserSessionResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.identity.v1.CreateBrowserSessionResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:user_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "userId")
  field(:session_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "sessionId")
  field(:expires_at, 3, type: Google.Protobuf.Timestamp, json_name: "expiresAt")
  field(:receipt, 4, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Identity.V1.RevokeBrowserSessionRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.identity.v1.RevokeBrowserSessionRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
end

defmodule Hephaestus.Identity.V1.RevokeBrowserSessionResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.identity.v1.RevokeBrowserSessionResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:receipt, 1, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Identity.V1.IdentityService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.identity.v1.IdentityService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :ResolveIdentity,
    Hephaestus.Identity.V1.ResolveIdentityRequest,
    Hephaestus.Identity.V1.ResolveIdentityResponse
  )

  rpc(
    :CreateBrowserSession,
    Hephaestus.Identity.V1.CreateBrowserSessionRequest,
    Hephaestus.Identity.V1.CreateBrowserSessionResponse
  )

  rpc(
    :RevokeBrowserSession,
    Hephaestus.Identity.V1.RevokeBrowserSessionRequest,
    Hephaestus.Identity.V1.RevokeBrowserSessionResponse
  )
end

defmodule Hephaestus.Identity.V1.IdentityService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Identity.V1.IdentityService.Service
end
