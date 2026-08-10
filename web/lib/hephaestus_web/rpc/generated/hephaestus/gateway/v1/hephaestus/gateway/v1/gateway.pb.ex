defmodule Hephaestus.Gateway.V1.GatewayLifecycle do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.gateway.v1.GatewayLifecycle",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:GATEWAY_LIFECYCLE_UNSPECIFIED, 0)
  field(:GATEWAY_LIFECYCLE_ENABLED, 1)
  field(:GATEWAY_LIFECYCLE_PAUSED, 2)
  field(:GATEWAY_LIFECYCLE_REMOVED, 3)
end

defmodule Hephaestus.Gateway.V1.GatewayIngressOutcome do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.gateway.v1.GatewayIngressOutcome",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:GATEWAY_INGRESS_OUTCOME_UNSPECIFIED, 0)
  field(:GATEWAY_INGRESS_OUTCOME_ACCEPTED, 1)
  field(:GATEWAY_INGRESS_OUTCOME_COMPLETED, 2)
  field(:GATEWAY_INGRESS_OUTCOME_FAILED, 3)
  field(:GATEWAY_INGRESS_OUTCOME_TIMED_OUT, 4)
  field(:GATEWAY_INGRESS_OUTCOME_REJECTED, 5)
end

defmodule Hephaestus.Gateway.V1.GatewayRoute do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.GatewayRoute",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:path, 2, type: :string)
  field(:methods, 3, repeated: true, type: :string)
  field(:enabled, 4, type: :bool)
end

defmodule Hephaestus.Gateway.V1.GatewayRevision do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.GatewayRevision",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:release_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:handler_contract, 3, type: :string, json_name: "handlerContract")
  field(:exposure, 4, type: :string)
  field(:secret_slots, 5, repeated: true, type: :string, json_name: "secretSlots")
  field(:created_at, 6, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:routes, 7, repeated: true, type: Hephaestus.Gateway.V1.GatewayRoute)
end

defmodule Hephaestus.Gateway.V1.GatewaySummary do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.GatewaySummary",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:project_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:repository_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:name, 4, type: :string)
  field(:lifecycle, 5, type: Hephaestus.Gateway.V1.GatewayLifecycle, enum: true)

  field(:active_revision_id, 6,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "activeRevisionId"
  )

  field(:updated_at, 7, type: Google.Protobuf.Timestamp, json_name: "updatedAt")
end

defmodule Hephaestus.Gateway.V1.GatewayIngress do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.GatewayIngress",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)

  field(:gateway_revision_id, 2,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "gatewayRevisionId"
  )

  field(:gateway_route_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "gatewayRouteId")
  field(:outcome, 4, type: Hephaestus.Gateway.V1.GatewayIngressOutcome, enum: true)
  field(:accepted_at, 5, type: Google.Protobuf.Timestamp, json_name: "acceptedAt")

  field(:completed_at, 6,
    proto3_optional: true,
    type: Google.Protobuf.Timestamp,
    json_name: "completedAt"
  )
end

defmodule Hephaestus.Gateway.V1.ListProjectGatewaysRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.ListProjectGatewaysRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Gateway.V1.ListProjectGatewaysResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.ListProjectGatewaysResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:gateways, 1, repeated: true, type: Hephaestus.Gateway.V1.GatewaySummary)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Gateway.V1.GetGatewayRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.GetGatewayRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:gateway_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "gatewayId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Gateway.V1.GetGatewayResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.GetGatewayResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:gateway, 1, type: Hephaestus.Gateway.V1.GatewaySummary)
  field(:revisions, 2, repeated: true, type: Hephaestus.Gateway.V1.GatewayRevision)
  field(:page, 3, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Gateway.V1.ListGatewayIngressRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.ListGatewayIngressRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:gateway_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "gatewayId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Gateway.V1.ListGatewayIngressResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.ListGatewayIngressResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:ingress, 1, repeated: true, type: Hephaestus.Gateway.V1.GatewayIngress)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Gateway.V1.SetGatewayLifecycleRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.SetGatewayLifecycleRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:gateway_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "gatewayId")
  field(:expected, 3, type: Hephaestus.Gateway.V1.GatewayLifecycle, enum: true)
  field(:next, 4, type: Hephaestus.Gateway.V1.GatewayLifecycle, enum: true)
end

defmodule Hephaestus.Gateway.V1.SetGatewayLifecycleResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.SetGatewayLifecycleResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:gateway, 1, type: Hephaestus.Gateway.V1.GatewaySummary)
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Gateway.V1.GatewayService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.gateway.v1.GatewayService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :ListProjectGateways,
    Hephaestus.Gateway.V1.ListProjectGatewaysRequest,
    Hephaestus.Gateway.V1.ListProjectGatewaysResponse
  )

  rpc(
    :GetGateway,
    Hephaestus.Gateway.V1.GetGatewayRequest,
    Hephaestus.Gateway.V1.GetGatewayResponse
  )

  rpc(
    :ListGatewayIngress,
    Hephaestus.Gateway.V1.ListGatewayIngressRequest,
    Hephaestus.Gateway.V1.ListGatewayIngressResponse
  )

  rpc(
    :SetGatewayLifecycle,
    Hephaestus.Gateway.V1.SetGatewayLifecycleRequest,
    Hephaestus.Gateway.V1.SetGatewayLifecycleResponse
  )
end

defmodule Hephaestus.Gateway.V1.GatewayService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Gateway.V1.GatewayService.Service
end
