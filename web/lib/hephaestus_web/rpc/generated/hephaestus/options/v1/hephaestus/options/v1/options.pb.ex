defmodule Hephaestus.Options.V1.ActorSource do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.options.v1.ActorSource",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:ACTOR_SOURCE_UNSPECIFIED, 0)
  field(:ACTOR_SOURCE_MEDIATOR_JWT_METADATA, 1)
  field(:ACTOR_SOURCE_VERIFIED_OIDC_BOOTSTRAP_ASSERTION, 2)
end

defmodule Hephaestus.Options.V1.OperationKind do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.options.v1.OperationKind",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:OPERATION_KIND_UNSPECIFIED, 0)
  field(:OPERATION_KIND_QUERY, 1)
  field(:OPERATION_KIND_MUTATION, 2)
  field(:OPERATION_KIND_SERVER_STREAM, 3)
end

defmodule Hephaestus.Options.V1.AuthorizationPolicy do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.options.v1.AuthorizationPolicy",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:permission, 1, type: :string)
  field(:audience, 2, type: :string)

  field(:actor_source, 3,
    type: Hephaestus.Options.V1.ActorSource,
    json_name: "actorSource",
    enum: true
  )
end
