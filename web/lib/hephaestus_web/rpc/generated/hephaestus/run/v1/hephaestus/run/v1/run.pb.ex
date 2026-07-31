defmodule Hephaestus.Run.V1.RunControlKind do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.run.v1.RunControlKind",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RUN_CONTROL_KIND_UNSPECIFIED, 0)
  field(:RUN_CONTROL_KIND_CANCEL, 1)
  field(:RUN_CONTROL_KIND_RETRY, 2)
  field(:RUN_CONTROL_KIND_APPROVE_RESULT, 3)
  field(:RUN_CONTROL_KIND_REJECT_RESULT, 4)
end

defmodule Hephaestus.Run.V1.ControlState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.run.v1.ControlState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:CONTROL_STATE_UNSPECIFIED, 0)
  field(:CONTROL_STATE_QUEUED, 1)
  field(:CONTROL_STATE_APPLIED, 2)
  field(:CONTROL_STATE_REJECTED, 3)
end

defmodule Hephaestus.Run.V1.RunSummary do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.RunSummary",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:state, 2, type: :string)
  field(:outcome, 3, type: :string)
  field(:run_kind, 4, type: :string, json_name: "runKind")
  field(:updated_at, 5, type: Google.Protobuf.Timestamp, json_name: "updatedAt")
  field(:instance_id, 6, type: Hephaestus.Common.V1.OpaqueId, json_name: "instanceId")
  field(:instance_name, 7, type: :string, json_name: "instanceName")
  field(:repository_id, 8, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:repository_name, 9, type: :string, json_name: "repositoryName")
  field(:commit_sha, 10, type: :string, json_name: "commitSha")
  field(:git_ref, 11, type: :string, json_name: "gitRef")
  field(:release_id, 12, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:release_version, 13, type: :string, json_name: "releaseVersion")

  field(:instance_revision_id, 14,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "instanceRevisionId"
  )
end

defmodule Hephaestus.Run.V1.RunFailure do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.RunFailure",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:code, 1, type: :string)
  field(:diagnostics, 2, repeated: true, type: Hephaestus.Common.V1.Diagnostic)
end

defmodule Hephaestus.Run.V1.RunEvent do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.RunEvent",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:payload, 0)

  field(:sequence, 1, type: :uint64)
  field(:event_type, 2, type: :string, json_name: "eventType")
  field(:bounded_log_message, 3, type: :string, json_name: "boundedLogMessage", oneof: 0)
  field(:state, 4, type: :string, oneof: 0)
  field(:metric, 5, type: Hephaestus.Common.V1.RuntimeMetric, oneof: 0)
  field(:diagnostic, 6, type: Hephaestus.Common.V1.Diagnostic, oneof: 0)
  field(:occurred_at, 7, type: Google.Protobuf.Timestamp, json_name: "occurredAt")
end

defmodule Hephaestus.Run.V1.RunMetrics do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.RunMetrics",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:event_count, 1, type: :uint64, json_name: "eventCount")
  field(:log_count, 2, type: :uint64, json_name: "logCount")
  field(:elapsed_ms, 3, type: :uint64, json_name: "elapsedMs")

  field(:runtime_metrics, 4,
    repeated: true,
    type: Hephaestus.Common.V1.RuntimeMetric,
    json_name: "runtimeMetrics"
  )
end

defmodule Hephaestus.Run.V1.ResultProposal do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.ResultProposal",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:state, 2, type: :string)
  field(:target_ref, 3, type: :string, json_name: "targetRef")
  field(:version, 4, type: :uint64)
end

defmodule Hephaestus.Run.V1.RunResult do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.RunResult",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:commit, 2, type: :string)
  field(:ref, 3, type: :string)
  field(:tree, 4, type: :string)
  field(:message, 5, type: :string)
  field(:artifact_manifest_hash, 6, type: :string, json_name: "artifactManifestHash")
  field(:proposal, 7, type: Hephaestus.Run.V1.ResultProposal)
end

defmodule Hephaestus.Run.V1.Run do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.Run",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:state, 2, type: :string)
  field(:outcome, 3, type: :string)
  field(:exit_code, 4, proto3_optional: true, type: :int32, json_name: "exitCode")
  field(:exit_signal, 5, proto3_optional: true, type: :int32, json_name: "exitSignal")
  field(:failure, 6, type: Hephaestus.Run.V1.RunFailure)
  field(:created_at, 7, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:updated_at, 8, type: Google.Protobuf.Timestamp, json_name: "updatedAt")
  field(:state_version, 9, type: :uint64, json_name: "stateVersion")
  field(:agent_id, 10, type: Hephaestus.Common.V1.OpaqueId, json_name: "agentId")
  field(:agent_name, 11, type: :string, json_name: "agentName")

  field(:instance_project_id, 12,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "instanceProjectId"
  )

  field(:instance_project_name, 13, type: :string, json_name: "instanceProjectName")

  field(:instance_revision_id, 14,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "instanceRevisionId"
  )

  field(:release_id, 15, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:release_version, 16, type: :string, json_name: "releaseVersion")

  field(:source_repository_id, 17,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "sourceRepositoryId"
  )

  field(:repository_id, 18, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:repository_name, 19, type: :string, json_name: "repositoryName")
  field(:project_id, 20, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:project_name, 21, type: :string, json_name: "projectName")
  field(:organization_id, 22, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:organization_name, 23, type: :string, json_name: "organizationName")
  field(:input_commit, 24, type: :string, json_name: "inputCommit")
  field(:git_ref, 25, type: :string, json_name: "gitRef")
  field(:attempt, 26, type: :uint32)
  field(:result, 27, type: Hephaestus.Run.V1.RunResult)
  field(:events, 28, repeated: true, type: Hephaestus.Run.V1.RunEvent)
  field(:artifacts, 29, repeated: true, type: Hephaestus.Artifact.V1.Artifact)
  field(:metrics, 30, type: Hephaestus.Run.V1.RunMetrics)
  field(:patch_preview, 31, proto3_optional: true, type: :string, json_name: "patchPreview")
  field(:manifest_preview, 32, proto3_optional: true, type: :string, json_name: "manifestPreview")
end

defmodule Hephaestus.Run.V1.ListProjectRunsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.ListProjectRunsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Run.V1.ListProjectRunsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.ListProjectRunsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:runs, 1, repeated: true, type: Hephaestus.Run.V1.RunSummary)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Run.V1.GetRunRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.GetRunRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:run_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "runId")
end

defmodule Hephaestus.Run.V1.GetRunResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.GetRunResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:run, 1, type: Hephaestus.Run.V1.Run)
end

defmodule Hephaestus.Run.V1.RunControlTarget do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.RunControlTarget",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:target, 0)

  field(:run_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "runId", oneof: 0)
  field(:proposal_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "proposalId", oneof: 0)
end

defmodule Hephaestus.Run.V1.RequestControlRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.RequestControlRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:kind, 2, type: Hephaestus.Run.V1.RunControlKind, enum: true)
  field(:repository_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:target, 4, type: Hephaestus.Run.V1.RunControlTarget)
  field(:reason, 5, type: :string)
end

defmodule Hephaestus.Run.V1.RequestControlResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.run.v1.RequestControlResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:control_request_id, 1,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "controlRequestId"
  )

  field(:state, 2, type: Hephaestus.Run.V1.ControlState, enum: true)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Run.V1.RunService.Service do
  @moduledoc false

  use GRPC.Service, name: "hephaestus.run.v1.RunService", protoc_gen_elixir_version: "0.17.0"

  rpc(
    :ListProjectRuns,
    Hephaestus.Run.V1.ListProjectRunsRequest,
    Hephaestus.Run.V1.ListProjectRunsResponse
  )

  rpc(:GetRun, Hephaestus.Run.V1.GetRunRequest, Hephaestus.Run.V1.GetRunResponse)

  rpc(
    :RequestControl,
    Hephaestus.Run.V1.RequestControlRequest,
    Hephaestus.Run.V1.RequestControlResponse
  )
end

defmodule Hephaestus.Run.V1.RunService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Run.V1.RunService.Service
end
