defmodule Hephaestus.Options.V1.PbExtension do
  @moduledoc false

  use Protobuf, protoc_gen_elixir_version: "0.17.0"

  extend(Google.Protobuf.FieldOptions, :sensitive, 51001, optional: true, type: :bool)

  extend(Google.Protobuf.MethodOptions, :authorization, 51002,
    optional: true,
    type: Hephaestus.Options.V1.AuthorizationPolicy
  )

  extend(Google.Protobuf.MethodOptions, :operation_kind, 51003,
    optional: true,
    type: Hephaestus.Options.V1.OperationKind,
    json_name: "operationKind",
    enum: true
  )

  extend(Google.Protobuf.MethodOptions, :max_request_bytes, 51004,
    optional: true,
    type: :uint64,
    json_name: "maxRequestBytes"
  )

  extend(Google.Protobuf.MethodOptions, :max_response_bytes, 51005,
    optional: true,
    type: :uint64,
    json_name: "maxResponseBytes"
  )
end
