defmodule Hephaestus.Pat.V1.GitOperation do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.pat.v1.GitOperation",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:GIT_OPERATION_UNSPECIFIED, 0)
  field(:GIT_OPERATION_DISCOVER, 1)
  field(:GIT_OPERATION_FETCH, 2)
  field(:GIT_OPERATION_RECEIVE, 3)
end

defmodule Hephaestus.Pat.V1.PersonalAccessTokenScope do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.pat.v1.PersonalAccessTokenScope",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:operations, 1, repeated: true, type: Hephaestus.Pat.V1.GitOperation, enum: true)

  field(:repository_ids, 2,
    repeated: true,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "repositoryIds"
  )
end

defmodule Hephaestus.Pat.V1.PersonalAccessTokenMetadata do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.pat.v1.PersonalAccessTokenMetadata",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:label, 2, type: :string)
  field(:scope, 3, type: Hephaestus.Pat.V1.PersonalAccessTokenScope)
  field(:created_at, 4, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:expires_at, 5, type: Google.Protobuf.Timestamp, json_name: "expiresAt")
  field(:revoked_at, 6, type: Google.Protobuf.Timestamp, json_name: "revokedAt")
  field(:last_used_at, 7, type: Google.Protobuf.Timestamp, json_name: "lastUsedAt")
end

defmodule Hephaestus.Pat.V1.PersonalAccessTokenValue do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.pat.v1.PersonalAccessTokenValue",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:value, 1, type: :bytes, deprecated: false)
end

defmodule Hephaestus.Pat.V1.ListPersonalAccessTokensRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.pat.v1.ListPersonalAccessTokensRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:page, 1, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Pat.V1.ListPersonalAccessTokensResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.pat.v1.ListPersonalAccessTokensResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:tokens, 1, repeated: true, type: Hephaestus.Pat.V1.PersonalAccessTokenMetadata)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Pat.V1.CreatePersonalAccessTokenRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.pat.v1.CreatePersonalAccessTokenRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:label, 2, type: :string)
  field(:scope, 3, type: Hephaestus.Pat.V1.PersonalAccessTokenScope)
  field(:expires_at, 4, type: Google.Protobuf.Timestamp, json_name: "expiresAt")
end

defmodule Hephaestus.Pat.V1.CreatePersonalAccessTokenResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.pat.v1.CreatePersonalAccessTokenResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:token, 1, type: Hephaestus.Pat.V1.PersonalAccessTokenMetadata)
  field(:value, 2, type: Hephaestus.Pat.V1.PersonalAccessTokenValue)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Pat.V1.RotatePersonalAccessTokenRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.pat.v1.RotatePersonalAccessTokenRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:token_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "tokenId")
  field(:label, 3, type: :string)
  field(:scope, 4, type: Hephaestus.Pat.V1.PersonalAccessTokenScope)
  field(:expires_at, 5, type: Google.Protobuf.Timestamp, json_name: "expiresAt")
end

defmodule Hephaestus.Pat.V1.RotatePersonalAccessTokenResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.pat.v1.RotatePersonalAccessTokenResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:token, 1, type: Hephaestus.Pat.V1.PersonalAccessTokenMetadata)
  field(:value, 2, type: Hephaestus.Pat.V1.PersonalAccessTokenValue)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Pat.V1.RevokePersonalAccessTokenRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.pat.v1.RevokePersonalAccessTokenRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:token_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "tokenId")
end

defmodule Hephaestus.Pat.V1.RevokePersonalAccessTokenResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.pat.v1.RevokePersonalAccessTokenResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:token, 1, type: Hephaestus.Pat.V1.PersonalAccessTokenMetadata)
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Pat.V1.PersonalAccessTokenService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.pat.v1.PersonalAccessTokenService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :ListPersonalAccessTokens,
    Hephaestus.Pat.V1.ListPersonalAccessTokensRequest,
    Hephaestus.Pat.V1.ListPersonalAccessTokensResponse
  )

  rpc(
    :CreatePersonalAccessToken,
    Hephaestus.Pat.V1.CreatePersonalAccessTokenRequest,
    Hephaestus.Pat.V1.CreatePersonalAccessTokenResponse
  )

  rpc(
    :RotatePersonalAccessToken,
    Hephaestus.Pat.V1.RotatePersonalAccessTokenRequest,
    Hephaestus.Pat.V1.RotatePersonalAccessTokenResponse
  )

  rpc(
    :RevokePersonalAccessToken,
    Hephaestus.Pat.V1.RevokePersonalAccessTokenRequest,
    Hephaestus.Pat.V1.RevokePersonalAccessTokenResponse
  )
end

defmodule Hephaestus.Pat.V1.PersonalAccessTokenService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Pat.V1.PersonalAccessTokenService.Service
end
