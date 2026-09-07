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
  field(:release_agent_id, 8, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseAgentId")
  field(:mailbox_slots, 9, repeated: true, type: :string, json_name: "mailboxSlots")
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

defmodule Hephaestus.Gateway.V1.GatewayMailboxBinding do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.GatewayMailboxBinding",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)

  field(:gateway_revision_id, 2,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "gatewayRevisionId"
  )

  field(:mailbox_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "mailboxId")
  field(:slot_key, 4, type: :string, json_name: "slotKey")
  field(:producer_id, 5, type: :string, json_name: "producerId")
  field(:grant_id, 6, type: Hephaestus.Common.V1.OpaqueId, json_name: "grantId")
  field(:grant_status, 7, type: :string, json_name: "grantStatus")
  field(:created_at, 8, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:granted_at, 9, type: Google.Protobuf.Timestamp, json_name: "grantedAt")

  field(:revoked_at, 10,
    proto3_optional: true,
    type: Google.Protobuf.Timestamp,
    json_name: "revokedAt"
  )
end

defmodule Hephaestus.Gateway.V1.GatewayMailboxPublication do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.GatewayMailboxPublication",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:invocation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "invocationId")

  field(:gateway_revision_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "gatewayRevisionId"
  )

  field(:binding_id, 4,
    proto3_optional: true,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "bindingId"
  )

  field(:grant_id, 5,
    proto3_optional: true,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "grantId"
  )

  field(:mailbox_id, 6,
    proto3_optional: true,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "mailboxId"
  )

  field(:event_id, 7,
    proto3_optional: true,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "eventId"
  )

  field(:slot_key, 8, type: :string, json_name: "slotKey")
  field(:outcome, 9, type: :string)
  field(:accepted_at, 10, type: Google.Protobuf.Timestamp, json_name: "acceptedAt")
  field(:settled_at, 11, type: Google.Protobuf.Timestamp, json_name: "settledAt")

  field(:authorization_snapshot_id, 12,
    proto3_optional: true,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "authorizationSnapshotId"
  )

  field(:snapshot_binding_ordinal, 13,
    proto3_optional: true,
    type: :uint32,
    json_name: "snapshotBindingOrdinal"
  )

  field(:delivery_disposition, 14, type: :string, json_name: "deliveryDisposition")
  field(:delivery_attempt_count, 15, type: :uint32, json_name: "deliveryAttemptCount")

  field(:delivery_terminal_at, 16,
    proto3_optional: true,
    type: Google.Protobuf.Timestamp,
    json_name: "deliveryTerminalAt"
  )

  field(:delivery_attempt_id, 17,
    proto3_optional: true,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "deliveryAttemptId"
  )

  field(:run_id, 18,
    proto3_optional: true,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "runId"
  )

  field(:run_state, 19, type: :string, json_name: "runState")
  field(:run_outcome, 20, type: :string, json_name: "runOutcome")
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

defmodule Hephaestus.Gateway.V1.InstallReleaseGatewaysRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.InstallReleaseGatewaysRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:release_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
end

defmodule Hephaestus.Gateway.V1.InstallReleaseGatewaysResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.InstallReleaseGatewaysResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:receipt, 1, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Gateway.V1.GatewaySecretSelection do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.GatewaySecretSelection",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:slot_key, 1, type: :string, json_name: "slotKey")
  field(:import_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "importId")
  field(:secret_version_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretVersionId")
  field(:route_path, 4, type: :string, json_name: "routePath")
  field(:header_name, 5, type: :string, json_name: "headerName")
end

defmodule Hephaestus.Gateway.V1.ConfigureGatewayRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.ConfigureGatewayRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:gateway_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "gatewayId")

  field(:expected_revision_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "expectedRevisionId"
  )

  field(:parameters, 4, repeated: true, type: Hephaestus.Common.V1.ParameterValue)

  field(:secret_selections, 5,
    repeated: true,
    type: Hephaestus.Gateway.V1.GatewaySecretSelection,
    json_name: "secretSelections"
  )
end

defmodule Hephaestus.Gateway.V1.ConfigureGatewayResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.ConfigureGatewayResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:revision_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "revisionId")
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
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

defmodule Hephaestus.Gateway.V1.CreateMailboxBindingRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.CreateMailboxBindingRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)

  field(:gateway_revision_id, 2,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "gatewayRevisionId"
  )

  field(:slot_key, 3, type: :string, json_name: "slotKey")
  field(:mailbox_id, 4, type: Hephaestus.Common.V1.OpaqueId, json_name: "mailboxId")
  field(:producer_id, 5, type: :string, json_name: "producerId")
end

defmodule Hephaestus.Gateway.V1.CreateMailboxBindingResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.CreateMailboxBindingResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:binding, 1, type: Hephaestus.Gateway.V1.GatewayMailboxBinding)
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Gateway.V1.RevokeMailboxBindingGrantRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.RevokeMailboxBindingGrantRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:binding_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "bindingId")
end

defmodule Hephaestus.Gateway.V1.RevokeMailboxBindingGrantResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.RevokeMailboxBindingGrantResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:binding, 1, type: Hephaestus.Gateway.V1.GatewayMailboxBinding)
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Gateway.V1.ListMailboxBindingsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.ListMailboxBindingsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:gateway_revision_id, 1,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "gatewayRevisionId"
  )

  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Gateway.V1.ListMailboxBindingsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.ListMailboxBindingsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:bindings, 1, repeated: true, type: Hephaestus.Gateway.V1.GatewayMailboxBinding)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Gateway.V1.ListMailboxPublicationsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.ListMailboxPublicationsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:gateway_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "gatewayId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Gateway.V1.ListMailboxPublicationsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.gateway.v1.ListMailboxPublicationsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:publications, 1, repeated: true, type: Hephaestus.Gateway.V1.GatewayMailboxPublication)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
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
    :InstallReleaseGateways,
    Hephaestus.Gateway.V1.InstallReleaseGatewaysRequest,
    Hephaestus.Gateway.V1.InstallReleaseGatewaysResponse
  )

  rpc(
    :ConfigureGateway,
    Hephaestus.Gateway.V1.ConfigureGatewayRequest,
    Hephaestus.Gateway.V1.ConfigureGatewayResponse
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

  rpc(
    :CreateMailboxBinding,
    Hephaestus.Gateway.V1.CreateMailboxBindingRequest,
    Hephaestus.Gateway.V1.CreateMailboxBindingResponse
  )

  rpc(
    :RevokeMailboxBindingGrant,
    Hephaestus.Gateway.V1.RevokeMailboxBindingGrantRequest,
    Hephaestus.Gateway.V1.RevokeMailboxBindingGrantResponse
  )

  rpc(
    :ListMailboxBindings,
    Hephaestus.Gateway.V1.ListMailboxBindingsRequest,
    Hephaestus.Gateway.V1.ListMailboxBindingsResponse
  )

  rpc(
    :ListMailboxPublications,
    Hephaestus.Gateway.V1.ListMailboxPublicationsRequest,
    Hephaestus.Gateway.V1.ListMailboxPublicationsResponse
  )
end

defmodule Hephaestus.Gateway.V1.GatewayService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Gateway.V1.GatewayService.Service
end
