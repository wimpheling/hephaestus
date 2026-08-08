defmodule Hephaestus.Instance.V1.TriggerPolicy do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.instance.v1.TriggerPolicy",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:TRIGGER_POLICY_UNSPECIFIED, 0)
  field(:TRIGGER_POLICY_MANUAL, 1)
  field(:TRIGGER_POLICY_PUSH, 2)
  field(:TRIGGER_POLICY_PUSH_AND_MANUAL, 3)
end

defmodule Hephaestus.Instance.V1.RecoveryAction do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.instance.v1.RecoveryAction",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RECOVERY_ACTION_UNSPECIFIED, 0)
  field(:RECOVERY_ACTION_RETRY, 1)
  field(:RECOVERY_ACTION_REJECT, 2)
  field(:RECOVERY_ACTION_RESUME, 3)
end

defmodule Hephaestus.Instance.V1.RecoveryDecision do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.instance.v1.RecoveryDecision",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RECOVERY_DECISION_UNSPECIFIED, 0)
  field(:RECOVERY_DECISION_RETRY_QUEUED, 1)
  field(:RECOVERY_DECISION_REJECTED, 2)
  field(:RECOVERY_DECISION_RESUMED, 3)
end

defmodule Hephaestus.Instance.V1.RemovalState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.instance.v1.RemovalState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:REMOVAL_STATE_UNSPECIFIED, 0)
  field(:REMOVAL_STATE_REMOVED, 1)
end

defmodule Hephaestus.Instance.V1.RefSelector do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.RefSelector",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:selector, 0)

  field(:exact, 1, type: :string, oneof: 0)
  field(:prefix, 2, type: :string, oneof: 0)
end

defmodule Hephaestus.Instance.V1.InstanceRevision do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.InstanceRevision",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:parameters, 2, repeated: true, type: Hephaestus.Common.V1.ParameterValue)
  field(:parameter_hash, 3, type: :string, json_name: "parameterHash")

  field(:resource_selection, 4,
    type: Hephaestus.Common.V1.RuntimePolicy,
    json_name: "resourceSelection"
  )

  field(:network_restriction, 5,
    type: Hephaestus.Common.V1.NetworkPolicy,
    json_name: "networkRestriction",
    enum: true
  )

  field(:effective_runtime_policy, 6,
    type: Hephaestus.Common.V1.RuntimePolicy,
    json_name: "effectiveRuntimePolicy"
  )

  field(:platform_policy_version, 7, type: :string, json_name: "platformPolicyVersion")
  field(:runnable, 8, type: :bool)
  field(:diagnostics, 9, repeated: true, type: Hephaestus.Common.V1.Diagnostic)
  field(:created_at, 10, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:release_agent_id, 11, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseAgentId")

  field(:parameter_schema, 12,
    repeated: true,
    type: Hephaestus.Common.V1.ParameterDeclaration,
    json_name: "parameterSchema"
  )

  field(:secret_slot_schema, 13,
    repeated: true,
    type: Hephaestus.Common.V1.SecretSlotDeclaration,
    json_name: "secretSlotSchema"
  )

  field(:runtime_contract, 14,
    type: Hephaestus.Common.V1.RuntimeContract,
    json_name: "runtimeContract"
  )

  field(:update_hook, 15, type: Hephaestus.Common.V1.UpdateHook, json_name: "updateHook")
  field(:release_id, 16, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:release_version, 17, type: :string, json_name: "releaseVersion")
  field(:release_state, 18, type: :string, json_name: "releaseState")
  field(:release_agent_name, 19, type: :string, json_name: "releaseAgentName")
end

defmodule Hephaestus.Instance.V1.CapabilityRequirement do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.CapabilityRequirement",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:release_agent_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseAgentId")
  field(:slot_key, 3, type: :string, json_name: "slotKey")
  field(:purpose, 4, type: :string)
  field(:resource_kind, 5, type: :string, json_name: "resourceKind")
  field(:required_operations, 6, repeated: true, type: :string, json_name: "requiredOperations")
  field(:optional_operations, 7, repeated: true, type: :string, json_name: "optionalOperations")
  field(:slot_required, 8, type: :bool, json_name: "slotRequired")
end

defmodule Hephaestus.Instance.V1.CapabilityResourceOption do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.CapabilityResourceOption",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:resource_kind, 2, type: :string, json_name: "resourceKind")
  field(:display_name, 3, type: :string, json_name: "displayName")
  field(:grantable_operations, 4, repeated: true, type: :string, json_name: "grantableOperations")
  field(:slot_key, 5, type: :string, json_name: "slotKey")
end

defmodule Hephaestus.Instance.V1.CapabilityBinding do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.CapabilityBinding",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)

  field(:instance_revision_id, 2,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "instanceRevisionId"
  )

  field(:requirement_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "requirementId")
  field(:slot_key, 4, type: :string, json_name: "slotKey")
  field(:resource_kind, 5, type: :string, json_name: "resourceKind")
  field(:resource_id, 6, type: Hephaestus.Common.V1.OpaqueId, json_name: "resourceId")
  field(:resource_name, 7, type: :string, json_name: "resourceName")
  field(:granted_operations, 8, repeated: true, type: :string, json_name: "grantedOperations")
  field(:grantor_id, 9, type: Hephaestus.Common.V1.OpaqueId, json_name: "grantorId")
  field(:grantor_name, 10, type: :string, json_name: "grantorName")
  field(:authorization_model_version, 11, type: :string, json_name: "authorizationModelVersion")
  field(:created_at, 12, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:live, 13, type: :bool)

  field(:last_used_at, 14,
    proto3_optional: true,
    type: Google.Protobuf.Timestamp,
    json_name: "lastUsedAt"
  )
end

defmodule Hephaestus.Instance.V1.RuntimeAuthoritySession do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.RuntimeAuthoritySession",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:run_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "runId")

  field(:instance_revision_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "instanceRevisionId"
  )

  field(:snapshot_id, 4, type: Hephaestus.Common.V1.OpaqueId, json_name: "snapshotId")
  field(:status, 5, type: :string)
  field(:issued_at, 6, type: Google.Protobuf.Timestamp, json_name: "issuedAt")
  field(:expires_at, 7, type: Google.Protobuf.Timestamp, json_name: "expiresAt")

  field(:acknowledged_at, 8,
    proto3_optional: true,
    type: Google.Protobuf.Timestamp,
    json_name: "acknowledgedAt"
  )

  field(:revoked_at, 9,
    proto3_optional: true,
    type: Google.Protobuf.Timestamp,
    json_name: "revokedAt"
  )

  field(:revocation_reason, 10, type: :string, json_name: "revocationReason")
end

defmodule Hephaestus.Instance.V1.CapabilityAuditRecord do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.CapabilityAuditRecord",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:run_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "runId")

  field(:runtime_session_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "runtimeSessionId"
  )

  field(:snapshot_id, 4, type: Hephaestus.Common.V1.OpaqueId, json_name: "snapshotId")
  field(:binding_id, 5, type: Hephaestus.Common.V1.OpaqueId, json_name: "bindingId")
  field(:slot_key, 6, type: :string, json_name: "slotKey")
  field(:resource_kind, 7, type: :string, json_name: "resourceKind")
  field(:resource_id, 8, type: Hephaestus.Common.V1.OpaqueId, json_name: "resourceId")
  field(:operation, 9, type: :string)
  field(:event_kind, 10, type: :string, json_name: "eventKind")
  field(:decision, 11, type: :string)
  field(:outcome, 12, type: :string)
  field(:reason_code, 13, type: :string, json_name: "reasonCode")
  field(:authorization_model_version, 14, type: :string, json_name: "authorizationModelVersion")
  field(:occurred_at, 15, type: Google.Protobuf.Timestamp, json_name: "occurredAt")
end

defmodule Hephaestus.Instance.V1.CapabilityMetrics do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.CapabilityMetrics",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:sessions_issued, 1, type: :uint64, json_name: "sessionsIssued")
  field(:sessions_active, 2, type: :uint64, json_name: "sessionsActive")
  field(:sessions_expired, 3, type: :uint64, json_name: "sessionsExpired")
  field(:sessions_revoked, 4, type: :uint64, json_name: "sessionsRevoked")
  field(:capability_calls, 5, type: :uint64, json_name: "capabilityCalls")
  field(:ceiling_denials, 6, type: :uint64, json_name: "ceilingDenials")
  field(:live_authorization_denials, 7, type: :uint64, json_name: "liveAuthorizationDenials")
  field(:invalid_revisions, 8, type: :uint64, json_name: "invalidRevisions")

  field(:average_revocation_latency_milliseconds, 9,
    type: :uint64,
    json_name: "averageRevocationLatencyMilliseconds"
  )
end

defmodule Hephaestus.Instance.V1.Attachment do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.Attachment",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:ref_selector, 2, type: Hephaestus.Instance.V1.RefSelector, json_name: "refSelector")

  field(:trigger_policy, 3,
    type: Hephaestus.Instance.V1.TriggerPolicy,
    json_name: "triggerPolicy",
    enum: true
  )

  field(:enabled, 4, type: :bool)
  field(:removed_at, 5, type: Google.Protobuf.Timestamp, json_name: "removedAt")
  field(:repository_id, 6, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:repository_name, 7, type: :string, json_name: "repositoryName")
  field(:can_manage, 8, type: :bool, json_name: "canManage")
end

defmodule Hephaestus.Instance.V1.UpdateEvent do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.UpdateEvent",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:payload, 0)

  field(:sequence, 1, type: :uint64)
  field(:event_type, 2, type: :string, json_name: "eventType")
  field(:bounded_log_message, 3, type: :string, json_name: "boundedLogMessage", oneof: 0)
  field(:diagnostic, 4, type: Hephaestus.Common.V1.Diagnostic, oneof: 0)

  field(:operation_state, 5,
    type: Hephaestus.Common.V1.OperationState,
    json_name: "operationState",
    enum: true,
    oneof: 0
  )
end

defmodule Hephaestus.Instance.V1.AgentUpdate do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.AgentUpdate",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)

  field(:expected_current_revision_id, 2,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "expectedCurrentRevisionId"
  )

  field(:candidate_revision_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "candidateRevisionId"
  )

  field(:state, 4, type: :string)
  field(:hook_run_id, 5, type: Hephaestus.Common.V1.OpaqueId, json_name: "hookRunId")
  field(:hook_exit_code, 6, proto3_optional: true, type: :int32, json_name: "hookExitCode")
  field(:hook_exit_signal, 7, proto3_optional: true, type: :int32, json_name: "hookExitSignal")
  field(:diagnostics, 8, repeated: true, type: Hephaestus.Common.V1.Diagnostic)

  field(:final_decision, 9,
    type: Hephaestus.Instance.V1.RecoveryDecision,
    json_name: "finalDecision",
    enum: true
  )

  field(:created_at, 10, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:updated_at, 11, type: Google.Protobuf.Timestamp, json_name: "updatedAt")

  field(:hook_events, 12,
    repeated: true,
    type: Hephaestus.Instance.V1.UpdateEvent,
    json_name: "hookEvents"
  )
end

defmodule Hephaestus.Instance.V1.RepositoryOption do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.RepositoryOption",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:name, 2, type: :string)
  field(:default_branch, 3, type: :string, json_name: "defaultBranch")
end

defmodule Hephaestus.Instance.V1.SecretImport do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.SecretImport",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:alias, 2, type: :string)
  field(:target, 3, type: Hephaestus.Secret.V1.SecretTarget)
  field(:state, 4, type: Hephaestus.Secret.V1.AuthorityState, enum: true)
  field(:secret_name, 5, type: :string, json_name: "secretName")

  field(:secret_state, 6,
    type: Hephaestus.Secret.V1.SecretState,
    json_name: "secretState",
    enum: true
  )

  field(:policy, 7, type: Hephaestus.Secret.V1.SecretPolicy)
  field(:expires_at, 8, type: Google.Protobuf.Timestamp, json_name: "expiresAt")
end

defmodule Hephaestus.Instance.V1.UpdateCandidate do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.UpdateCandidate",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:display_name, 2, type: :string, json_name: "displayName")

  field(:parameter_schema, 3,
    repeated: true,
    type: Hephaestus.Common.V1.ParameterDeclaration,
    json_name: "parameterSchema"
  )

  field(:secret_slot_schema, 4,
    repeated: true,
    type: Hephaestus.Common.V1.SecretSlotDeclaration,
    json_name: "secretSlotSchema"
  )

  field(:runtime_contract, 5,
    type: Hephaestus.Common.V1.RuntimeContract,
    json_name: "runtimeContract"
  )

  field(:requires_state, 6, type: :bool, json_name: "requiresState")
  field(:update_hook, 7, type: Hephaestus.Common.V1.UpdateHook, json_name: "updateHook")
  field(:release_id, 8, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:release_version, 9, type: :string, json_name: "releaseVersion")

  field(:capability_requirements, 10,
    repeated: true,
    type: Hephaestus.Instance.V1.CapabilityRequirement,
    json_name: "capabilityRequirements"
  )
end

defmodule Hephaestus.Instance.V1.RecentRun do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.RecentRun",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:state, 2, type: :string)
  field(:outcome, 3, type: :string)
  field(:run_kind, 4, type: :string, json_name: "runKind")

  field(:instance_revision_id, 5,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "instanceRevisionId"
  )

  field(:release_id, 6, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:attachment_id, 7, type: Hephaestus.Common.V1.OpaqueId, json_name: "attachmentId")
  field(:created_at, 8, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:updated_at, 9, type: Google.Protobuf.Timestamp, json_name: "updatedAt")
end

defmodule Hephaestus.Instance.V1.AgentInstance do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.AgentInstance",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:name, 2, type: :string)
  field(:state, 3, type: :string)
  field(:run_gate_open, 4, type: :bool, json_name: "runGateOpen")

  field(:active_revision_id, 5,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "activeRevisionId"
  )

  field(:state_volume_id, 6, type: Hephaestus.Common.V1.OpaqueId, json_name: "stateVolumeId")
  field(:created_at, 7, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:updated_at, 8, type: Google.Protobuf.Timestamp, json_name: "updatedAt")
  field(:project_id, 9, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:project_name, 10, type: :string, json_name: "projectName")
  field(:organization_id, 11, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:organization_name, 12, type: :string, json_name: "organizationName")
  field(:can_manage, 13, type: :bool, json_name: "canManage")
  field(:can_update, 14, type: :bool, json_name: "canUpdate")
  field(:can_recover, 15, type: :bool, json_name: "canRecover")
  field(:revisions, 16, repeated: true, type: Hephaestus.Instance.V1.InstanceRevision)
  field(:attachments, 17, repeated: true, type: Hephaestus.Instance.V1.Attachment)
  field(:updates, 18, repeated: true, type: Hephaestus.Instance.V1.AgentUpdate)
  field(:repositories, 19, repeated: true, type: Hephaestus.Instance.V1.RepositoryOption)

  field(:secret_imports, 20,
    repeated: true,
    type: Hephaestus.Instance.V1.SecretImport,
    json_name: "secretImports"
  )

  field(:update_candidates, 21,
    repeated: true,
    type: Hephaestus.Instance.V1.UpdateCandidate,
    json_name: "updateCandidates"
  )

  field(:recent_runs, 22,
    repeated: true,
    type: Hephaestus.Instance.V1.RecentRun,
    json_name: "recentRuns"
  )

  field(:capability_requirements, 23,
    repeated: true,
    type: Hephaestus.Instance.V1.CapabilityRequirement,
    json_name: "capabilityRequirements"
  )

  field(:capability_resource_options, 24,
    repeated: true,
    type: Hephaestus.Instance.V1.CapabilityResourceOption,
    json_name: "capabilityResourceOptions"
  )

  field(:capability_bindings, 25,
    repeated: true,
    type: Hephaestus.Instance.V1.CapabilityBinding,
    json_name: "capabilityBindings"
  )

  field(:runtime_sessions, 26,
    repeated: true,
    type: Hephaestus.Instance.V1.RuntimeAuthoritySession,
    json_name: "runtimeSessions"
  )

  field(:capability_audit, 27,
    repeated: true,
    type: Hephaestus.Instance.V1.CapabilityAuditRecord,
    json_name: "capabilityAudit"
  )

  field(:capability_metrics, 28,
    type: Hephaestus.Instance.V1.CapabilityMetrics,
    json_name: "capabilityMetrics"
  )
end

defmodule Hephaestus.Instance.V1.GetInstanceRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.GetInstanceRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:instance_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "instanceId")
end

defmodule Hephaestus.Instance.V1.GetInstanceResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.GetInstanceResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:instance, 1, type: Hephaestus.Instance.V1.AgentInstance)
end

defmodule Hephaestus.Instance.V1.ImportAgentRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.ImportAgentRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:project_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:release_agent_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseAgentId")
  field(:name, 4, type: :string)
  field(:parameters, 5, repeated: true, type: Hephaestus.Common.V1.ParameterValue)

  field(:selected_policy, 6,
    type: Hephaestus.Common.V1.RuntimePolicy,
    json_name: "selectedPolicy"
  )
end

defmodule Hephaestus.Instance.V1.ImportAgentResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.ImportAgentResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:instance_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "instanceId")
  field(:revision_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "revisionId")
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Instance.V1.CreateAttachmentRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.CreateAttachmentRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:instance_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "instanceId")
  field(:repository_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:ref_selector, 4, type: Hephaestus.Instance.V1.RefSelector, json_name: "refSelector")

  field(:trigger_policy, 5,
    type: Hephaestus.Instance.V1.TriggerPolicy,
    json_name: "triggerPolicy",
    enum: true
  )
end

defmodule Hephaestus.Instance.V1.CreateAttachmentResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.CreateAttachmentResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:attachment_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "attachmentId")
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Instance.V1.SetAttachmentEnabledRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.SetAttachmentEnabledRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:attachment_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "attachmentId")
  field(:enabled, 3, type: :bool)
end

defmodule Hephaestus.Instance.V1.SetAttachmentEnabledResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.SetAttachmentEnabledResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:attachment_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "attachmentId")
  field(:enabled, 2, type: :bool)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Instance.V1.RemoveAttachmentRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.RemoveAttachmentRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:attachment_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "attachmentId")
end

defmodule Hephaestus.Instance.V1.RemoveAttachmentResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.RemoveAttachmentResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:attachment_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "attachmentId")
  field(:state, 2, type: Hephaestus.Instance.V1.RemovalState, enum: true)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Instance.V1.ReviseInstanceRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.ReviseInstanceRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:instance_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "instanceId")

  field(:expected_revision_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "expectedRevisionId"
  )

  field(:parameters, 4, repeated: true, type: Hephaestus.Common.V1.ParameterValue)

  field(:selected_policy, 5,
    type: Hephaestus.Common.V1.RuntimePolicy,
    json_name: "selectedPolicy"
  )
end

defmodule Hephaestus.Instance.V1.ReviseInstanceResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.ReviseInstanceResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:instance_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "instanceId")
  field(:revision_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "revisionId")
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Instance.V1.CreateUpdateRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.CreateUpdateRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:instance_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "instanceId")

  field(:expected_revision_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "expectedRevisionId"
  )

  field(:candidate_release_agent_id, 4,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "candidateReleaseAgentId"
  )

  field(:parameters, 5, repeated: true, type: Hephaestus.Common.V1.ParameterValue)

  field(:selected_policy, 6,
    type: Hephaestus.Common.V1.RuntimePolicy,
    json_name: "selectedPolicy"
  )
end

defmodule Hephaestus.Instance.V1.CreateUpdateResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.CreateUpdateResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:update_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "updateId")

  field(:candidate_revision_id, 2,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "candidateRevisionId"
  )

  field(:hook_run_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "hookRunId")
  field(:operation, 4, type: Hephaestus.Common.V1.Operation)
  field(:receipt, 5, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Instance.V1.RecoverUpdateRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.RecoverUpdateRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:update_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "updateId")
  field(:action, 3, type: Hephaestus.Instance.V1.RecoveryAction, enum: true)
end

defmodule Hephaestus.Instance.V1.RecoverUpdateResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.RecoverUpdateResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:update_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "updateId")
  field(:decision, 2, type: Hephaestus.Instance.V1.RecoveryDecision, enum: true)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Instance.V1.BindSecretRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.BindSecretRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:instance_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "instanceId")

  field(:expected_revision_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "expectedRevisionId"
  )

  field(:import_id, 4, type: Hephaestus.Common.V1.OpaqueId, json_name: "importId")
  field(:slot, 5, type: :string)
  field(:mode, 6, type: Hephaestus.Secret.V1.DeliveryMode, enum: true)
  field(:phases, 7, repeated: true, type: Hephaestus.Secret.V1.DeliveryPhase, enum: true)

  field(:attachment_ids, 8,
    repeated: true,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "attachmentIds"
  )

  field(:destinations, 9, repeated: true, type: :string)
end

defmodule Hephaestus.Instance.V1.BindSecretResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.BindSecretResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:binding_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "bindingId")

  field(:instance_revision_id, 2,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "instanceRevisionId"
  )

  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Instance.V1.CapabilityBindingSelection do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.CapabilityBindingSelection",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:slot_key, 1, type: :string, json_name: "slotKey")
  field(:resource_kind, 2, type: :string, json_name: "resourceKind")
  field(:resource_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "resourceId")
  field(:granted_operations, 4, repeated: true, type: :string, json_name: "grantedOperations")
end

defmodule Hephaestus.Instance.V1.ReviseCapabilitiesRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.ReviseCapabilitiesRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:instance_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "instanceId")

  field(:expected_revision_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "expectedRevisionId"
  )

  field(:bindings, 4, repeated: true, type: Hephaestus.Instance.V1.CapabilityBindingSelection)
end

defmodule Hephaestus.Instance.V1.ReviseCapabilitiesResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.instance.v1.ReviseCapabilitiesResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:instance_revision_id, 1,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "instanceRevisionId"
  )

  field(:runnable, 2, type: :bool)
  field(:diagnostics, 3, repeated: true, type: Hephaestus.Common.V1.Diagnostic)
  field(:receipt, 4, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Instance.V1.AgentInstanceService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.instance.v1.AgentInstanceService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :GetInstance,
    Hephaestus.Instance.V1.GetInstanceRequest,
    Hephaestus.Instance.V1.GetInstanceResponse
  )

  rpc(
    :ImportAgent,
    Hephaestus.Instance.V1.ImportAgentRequest,
    Hephaestus.Instance.V1.ImportAgentResponse
  )

  rpc(
    :CreateAttachment,
    Hephaestus.Instance.V1.CreateAttachmentRequest,
    Hephaestus.Instance.V1.CreateAttachmentResponse
  )

  rpc(
    :SetAttachmentEnabled,
    Hephaestus.Instance.V1.SetAttachmentEnabledRequest,
    Hephaestus.Instance.V1.SetAttachmentEnabledResponse
  )

  rpc(
    :RemoveAttachment,
    Hephaestus.Instance.V1.RemoveAttachmentRequest,
    Hephaestus.Instance.V1.RemoveAttachmentResponse
  )

  rpc(
    :ReviseInstance,
    Hephaestus.Instance.V1.ReviseInstanceRequest,
    Hephaestus.Instance.V1.ReviseInstanceResponse
  )

  rpc(
    :CreateUpdate,
    Hephaestus.Instance.V1.CreateUpdateRequest,
    Hephaestus.Instance.V1.CreateUpdateResponse
  )

  rpc(
    :RecoverUpdate,
    Hephaestus.Instance.V1.RecoverUpdateRequest,
    Hephaestus.Instance.V1.RecoverUpdateResponse
  )

  rpc(
    :BindSecret,
    Hephaestus.Instance.V1.BindSecretRequest,
    Hephaestus.Instance.V1.BindSecretResponse
  )

  rpc(
    :ReviseCapabilities,
    Hephaestus.Instance.V1.ReviseCapabilitiesRequest,
    Hephaestus.Instance.V1.ReviseCapabilitiesResponse
  )
end

defmodule Hephaestus.Instance.V1.AgentInstanceService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Instance.V1.AgentInstanceService.Service
end
