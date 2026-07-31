defmodule Hephaestus.Build.V1.BuildState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.build.v1.BuildState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:BUILD_STATE_UNSPECIFIED, 0)
  field(:BUILD_STATE_QUEUED, 1)
  field(:BUILD_STATE_RUNNING, 2)
  field(:BUILD_STATE_SUCCEEDED, 3)
  field(:BUILD_STATE_FAILED, 4)
  field(:BUILD_STATE_CANCELLED, 5)
end

defmodule Hephaestus.Build.V1.Build do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.Build",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:state, 2, type: Hephaestus.Build.V1.BuildState, enum: true)
  field(:exit_code, 3, proto3_optional: true, type: :int32, json_name: "exitCode")
  field(:failure_code, 4, type: :string, json_name: "failureCode")
  field(:logs, 5, repeated: true, type: :string)
  field(:metrics, 6, repeated: true, type: Hephaestus.Common.V1.RuntimeMetric)
  field(:created_at, 7, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:updated_at, 8, type: Google.Protobuf.Timestamp, json_name: "updatedAt")
end

defmodule Hephaestus.Build.V1.GetBuildRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.GetBuildRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:build_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildId")
end

defmodule Hephaestus.Build.V1.GetBuildResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.GetBuildResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:build, 1, type: Hephaestus.Build.V1.Build)
end

defmodule Hephaestus.Build.V1.RequestBuildRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.RequestBuildRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:repository_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:source_commit, 3, type: :string, json_name: "sourceCommit")
  field(:build_definition_hash, 4, type: :string, json_name: "buildDefinitionHash")
  field(:configuration_hash, 5, type: :string, json_name: "configurationHash")
end

defmodule Hephaestus.Build.V1.RequestBuildResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.RequestBuildResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:build_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildId")
  field(:operation, 2, type: Hephaestus.Common.V1.Operation)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Build.V1.BuildService.Service do
  @moduledoc false

  use GRPC.Service, name: "hephaestus.build.v1.BuildService", protoc_gen_elixir_version: "0.17.0"

  rpc(:GetBuild, Hephaestus.Build.V1.GetBuildRequest, Hephaestus.Build.V1.GetBuildResponse)

  rpc(
    :RequestBuild,
    Hephaestus.Build.V1.RequestBuildRequest,
    Hephaestus.Build.V1.RequestBuildResponse
  )
end

defmodule Hephaestus.Build.V1.BuildService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Build.V1.BuildService.Service
end
