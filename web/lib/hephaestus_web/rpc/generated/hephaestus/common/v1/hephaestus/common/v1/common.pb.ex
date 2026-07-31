defmodule Hephaestus.Common.V1.NetworkPolicy do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.common.v1.NetworkPolicy",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:NETWORK_POLICY_UNSPECIFIED, 0)
  field(:NETWORK_POLICY_DISABLED, 1)
  field(:NETWORK_POLICY_BROKER_ONLY, 2)
  field(:NETWORK_POLICY_EGRESS, 3)
end

defmodule Hephaestus.Common.V1.DiagnosticSeverity do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.common.v1.DiagnosticSeverity",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:DIAGNOSTIC_SEVERITY_UNSPECIFIED, 0)
  field(:DIAGNOSTIC_SEVERITY_INFO, 1)
  field(:DIAGNOSTIC_SEVERITY_WARNING, 2)
  field(:DIAGNOSTIC_SEVERITY_ERROR, 3)
end

defmodule Hephaestus.Common.V1.DiagnosticCode do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.common.v1.DiagnosticCode",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:DIAGNOSTIC_CODE_UNSPECIFIED, 0)
  field(:DIAGNOSTIC_CODE_INVALID_PARAMETER, 1)
  field(:DIAGNOSTIC_CODE_POLICY_EXCEEDED, 2)
  field(:DIAGNOSTIC_CODE_INCOMPATIBLE_UPDATE, 3)
  field(:DIAGNOSTIC_CODE_RUN_GATE_CLOSED, 4)
  field(:DIAGNOSTIC_CODE_RESOURCE_UNAVAILABLE, 5)
end

defmodule Hephaestus.Common.V1.ErrorCode do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.common.v1.ErrorCode",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:ERROR_CODE_UNSPECIFIED, 0)
  field(:ERROR_CODE_UNAUTHENTICATED, 1)
  field(:ERROR_CODE_PERMISSION_DENIED, 2)
  field(:ERROR_CODE_NOT_FOUND, 3)
  field(:ERROR_CODE_INVALID_ARGUMENT, 4)
  field(:ERROR_CODE_STALE_VERSION, 5)
  field(:ERROR_CODE_IDEMPOTENCY_CONFLICT, 6)
  field(:ERROR_CODE_LIFECYCLE_CONFLICT, 7)
  field(:ERROR_CODE_RESOURCE_EXHAUSTED, 8)
  field(:ERROR_CODE_UNAVAILABLE, 9)
  field(:ERROR_CODE_INTERNAL, 10)
  field(:ERROR_CODE_CURSOR_EXPIRED, 11)
  field(:ERROR_CODE_ALREADY_EXISTS, 12)
  field(:ERROR_CODE_DEADLINE_EXCEEDED, 13)
  field(:ERROR_CODE_CANCELLED, 14)
end

defmodule Hephaestus.Common.V1.OperationState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.common.v1.OperationState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:OPERATION_STATE_UNSPECIFIED, 0)
  field(:OPERATION_STATE_QUEUED, 1)
  field(:OPERATION_STATE_RUNNING, 2)
  field(:OPERATION_STATE_SUCCEEDED, 3)
  field(:OPERATION_STATE_FAILED, 4)
  field(:OPERATION_STATE_CANCELLED, 5)
end

defmodule Hephaestus.Common.V1.SecretSlotDeliveryMode do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.common.v1.SecretSlotDeliveryMode",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:SECRET_SLOT_DELIVERY_MODE_UNSPECIFIED, 0)
  field(:SECRET_SLOT_DELIVERY_MODE_RAW, 1)
  field(:SECRET_SLOT_DELIVERY_MODE_BROKERED, 2)
end

defmodule Hephaestus.Common.V1.SecretSlotPhase do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.common.v1.SecretSlotPhase",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:SECRET_SLOT_PHASE_UNSPECIFIED, 0)
  field(:SECRET_SLOT_PHASE_NORMAL, 1)
  field(:SECRET_SLOT_PHASE_UPDATE, 2)
end

defmodule Hephaestus.Common.V1.OpaqueId do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.OpaqueId",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:value, 1, type: :string)
end

defmodule Hephaestus.Common.V1.RequestContext do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.RequestContext",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:request_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "requestId")
  field(:idempotency_key, 2, type: :string, json_name: "idempotencyKey")
end

defmodule Hephaestus.Common.V1.PageRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.PageRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:page_size, 1, type: :uint32, json_name: "pageSize")
  field(:page_token, 2, type: :string, json_name: "pageToken")
end

defmodule Hephaestus.Common.V1.PageResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.PageResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:next_page_token, 1, type: :string, json_name: "nextPageToken")
  field(:stable_order, 2, type: :string, json_name: "stableOrder")
end

defmodule Hephaestus.Common.V1.Cursor do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.Cursor",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:value, 1, type: :string)
end

defmodule Hephaestus.Common.V1.MutationReceipt do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.MutationReceipt",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:committed_cursor, 1, type: Hephaestus.Common.V1.Cursor, json_name: "committedCursor")
  field(:aggregate_version, 2, type: :uint64, json_name: "aggregateVersion")
  field(:event_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "eventId")
end

defmodule Hephaestus.Common.V1.ParameterValue do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.ParameterValue",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:value, 0)

  field(:name, 1, type: :string)
  field(:string_value, 2, type: :string, json_name: "stringValue", oneof: 0)
  field(:integer_value, 3, type: :int64, json_name: "integerValue", oneof: 0)
  field(:boolean_value, 4, type: :bool, json_name: "booleanValue", oneof: 0)
end

defmodule Hephaestus.Common.V1.StringParameterConstraints do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.StringParameterConstraints",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:minimum_length, 1, type: :uint32, json_name: "minimumLength")
  field(:maximum_length, 2, type: :uint32, json_name: "maximumLength")
end

defmodule Hephaestus.Common.V1.IntegerParameterConstraints do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.IntegerParameterConstraints",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:minimum, 1, type: :int64)
  field(:maximum, 2, type: :int64)
end

defmodule Hephaestus.Common.V1.BooleanParameterConstraints do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.BooleanParameterConstraints",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3
end

defmodule Hephaestus.Common.V1.EnumParameterConstraints do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.EnumParameterConstraints",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:values, 1, repeated: true, type: :string)
end

defmodule Hephaestus.Common.V1.ParameterType do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.ParameterType",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:constraint, 0)

  field(:string, 1, type: Hephaestus.Common.V1.StringParameterConstraints, oneof: 0)
  field(:integer, 2, type: Hephaestus.Common.V1.IntegerParameterConstraints, oneof: 0)
  field(:boolean, 3, type: Hephaestus.Common.V1.BooleanParameterConstraints, oneof: 0)
  field(:enumeration, 4, type: Hephaestus.Common.V1.EnumParameterConstraints, oneof: 0)
end

defmodule Hephaestus.Common.V1.ParameterDefault do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.ParameterDefault",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:value, 0)

  field(:string_value, 1, type: :string, json_name: "stringValue", oneof: 0)
  field(:integer_value, 2, type: :int64, json_name: "integerValue", oneof: 0)
  field(:boolean_value, 3, type: :bool, json_name: "booleanValue", oneof: 0)
end

defmodule Hephaestus.Common.V1.ParameterDeclaration do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.ParameterDeclaration",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:name, 1, type: :string)
  field(:label, 2, type: :string)
  field(:value_type, 3, type: Hephaestus.Common.V1.ParameterType, json_name: "valueType")
  field(:required, 4, type: :bool)
  field(:default, 5, type: Hephaestus.Common.V1.ParameterDefault)
  field(:sensitive, 6, type: :bool)
end

defmodule Hephaestus.Common.V1.RuntimePolicy do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.RuntimePolicy",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:vcpus, 1, type: :uint32)
  field(:memory_mib, 2, type: :uint32, json_name: "memoryMib")
  field(:network, 3, type: Hephaestus.Common.V1.NetworkPolicy, enum: true)
end

defmodule Hephaestus.Common.V1.RuntimeContract do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.RuntimeContract",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:policy_ceiling, 1, type: Hephaestus.Common.V1.RuntimePolicy, json_name: "policyCeiling")
  field(:platform_policy_version, 2, type: :string, json_name: "platformPolicyVersion")
  field(:requires_state, 3, type: :bool, json_name: "requiresState")
end

defmodule Hephaestus.Common.V1.Diagnostic do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.Diagnostic",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:code, 1, type: Hephaestus.Common.V1.DiagnosticCode, enum: true)
  field(:severity, 2, type: Hephaestus.Common.V1.DiagnosticSeverity, enum: true)
  field(:field, 3, type: :string)
  field(:message, 4, type: :string)
end

defmodule Hephaestus.Common.V1.ErrorDetail do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.ErrorDetail",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:code, 1, type: Hephaestus.Common.V1.ErrorCode, enum: true)
  field(:reason, 2, type: :string)
  field(:diagnostics, 3, repeated: true, type: Hephaestus.Common.V1.Diagnostic)
  field(:request_id, 4, type: Hephaestus.Common.V1.OpaqueId, json_name: "requestId")
  field(:retryable, 5, type: :bool)
end

defmodule Hephaestus.Common.V1.Operation do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.Operation",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:state, 2, type: Hephaestus.Common.V1.OperationState, enum: true)
  field(:created_at, 3, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:updated_at, 4, type: Google.Protobuf.Timestamp, json_name: "updatedAt")
  field(:diagnostics, 5, repeated: true, type: Hephaestus.Common.V1.Diagnostic)
end

defmodule Hephaestus.Common.V1.SecretSlotDeclaration do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.SecretSlotDeclaration",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:key, 1, type: :string)
  field(:purpose, 2, type: :string)
  field(:required, 3, type: :bool)

  field(:delivery_modes, 4,
    repeated: true,
    type: Hephaestus.Common.V1.SecretSlotDeliveryMode,
    json_name: "deliveryModes",
    enum: true
  )

  field(:phases, 5, repeated: true, type: Hephaestus.Common.V1.SecretSlotPhase, enum: true)
  field(:destinations, 6, repeated: true, type: :string)
end

defmodule Hephaestus.Common.V1.UpdateHook do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.UpdateHook",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:required, 1, type: :bool)
  field(:timeout_seconds, 2, type: :uint32, json_name: "timeoutSeconds")
end

defmodule Hephaestus.Common.V1.MetricLabel do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.MetricLabel",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:key, 1, type: :string)
  field(:value, 2, type: :string)
end

defmodule Hephaestus.Common.V1.RuntimeMetric do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.common.v1.RuntimeMetric",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:name, 1, type: :string)
  field(:value, 2, type: :double)
  field(:unit, 3, type: :string)
  field(:labels, 4, repeated: true, type: Hephaestus.Common.V1.MetricLabel)
end
