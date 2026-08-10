defmodule Hephaestus.Release.V1.ReleaseState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.release.v1.ReleaseState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RELEASE_STATE_UNSPECIFIED, 0)
  field(:RELEASE_STATE_DRAFT, 1)
  field(:RELEASE_STATE_PUBLISHED, 2)
  field(:RELEASE_STATE_REVOKED, 3)
end

defmodule Hephaestus.Release.V1.ReleaseSummary do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ReleaseSummary",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:version, 2, type: :string)
  field(:state, 3, type: Hephaestus.Release.V1.ReleaseState, enum: true)
  field(:source_commit, 4, type: :string, json_name: "sourceCommit")
  field(:source_ref, 5, type: :string, json_name: "sourceRef")
  field(:build_request_id, 6, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildRequestId")
  field(:created_at, 7, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:published_at, 8, type: Google.Protobuf.Timestamp, json_name: "publishedAt")
  field(:manifest_hash, 9, type: :string, json_name: "manifestHash")
  field(:artifact_count, 10, type: :uint32, json_name: "artifactCount")
  field(:exported_agent_count, 11, type: :uint32, json_name: "exportedAgentCount")
end

defmodule Hephaestus.Release.V1.ReleaseAgent do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ReleaseAgent",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:family_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "familyId")
  field(:agent_key, 3, type: :string, json_name: "agentKey")
  field(:display_name, 4, type: :string, json_name: "displayName")

  field(:runtime_contract, 5,
    type: Hephaestus.Common.V1.RuntimeContract,
    json_name: "runtimeContract"
  )

  field(:parameter_schema, 6,
    repeated: true,
    type: Hephaestus.Common.V1.ParameterDeclaration,
    json_name: "parameterSchema"
  )

  field(:secret_slot_schema, 7,
    repeated: true,
    type: Hephaestus.Common.V1.SecretSlotDeclaration,
    json_name: "secretSlotSchema"
  )

  field(:requires_state, 8, type: :bool, json_name: "requiresState")
  field(:update_hook, 9, type: Hephaestus.Common.V1.UpdateHook, json_name: "updateHook")
  field(:created_at, 10, type: Google.Protobuf.Timestamp, json_name: "createdAt")
end

defmodule Hephaestus.Release.V1.Release do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.Release",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:version, 2, type: :string)
  field(:state, 3, type: Hephaestus.Release.V1.ReleaseState, enum: true)
  field(:source_commit, 4, type: :string, json_name: "sourceCommit")
  field(:source_ref, 5, type: :string, json_name: "sourceRef")
  field(:build_request_id, 6, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildRequestId")
  field(:build_definition_hash, 7, type: :string, json_name: "buildDefinitionHash")
  field(:configuration_hash, 8, type: :string, json_name: "configurationHash")
  field(:manifest_hash, 9, type: :string, json_name: "manifestHash")
  field(:created_at, 10, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:published_at, 11, type: Google.Protobuf.Timestamp, json_name: "publishedAt")
  field(:revoked_at, 12, type: Google.Protobuf.Timestamp, json_name: "revokedAt")
  field(:repository_id, 13, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:repository_name, 14, type: :string, json_name: "repositoryName")
  field(:project_id, 15, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:project_name, 16, type: :string, json_name: "projectName")
  field(:organization_id, 17, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:organization_name, 18, type: :string, json_name: "organizationName")
  field(:build, 19, type: Hephaestus.Build.V1.Build)
  field(:artifacts, 20, repeated: true, type: Hephaestus.Artifact.V1.Artifact)
  field(:agents, 21, repeated: true, type: Hephaestus.Release.V1.ReleaseAgent)
end

defmodule Hephaestus.Release.V1.ListRepositoryReleasesRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ListRepositoryReleasesRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Release.V1.ListRepositoryReleasesResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ListRepositoryReleasesResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:releases, 1, repeated: true, type: Hephaestus.Release.V1.ReleaseSummary)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Release.V1.GetReleaseRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.GetReleaseRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:release_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
end

defmodule Hephaestus.Release.V1.GetReleaseResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.GetReleaseResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:release, 1, type: Hephaestus.Release.V1.Release)
end

defmodule Hephaestus.Release.V1.SetDraftVersionRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.SetDraftVersionRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:release_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:version, 3, type: :string)
end

defmodule Hephaestus.Release.V1.SetDraftVersionResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.SetDraftVersionResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:release, 1, type: Hephaestus.Release.V1.Release)
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Release.V1.PublishReleaseRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.PublishReleaseRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:release_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
end

defmodule Hephaestus.Release.V1.PublishReleaseResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.PublishReleaseResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:release, 1, type: Hephaestus.Release.V1.Release)
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Release.V1.WatchReleaseRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.WatchReleaseRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:release_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:resume_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_events, 3, type: :uint32, json_name: "maxEvents")
  field(:max_total_bytes, 4, type: :uint64, json_name: "maxTotalBytes")
end

defmodule Hephaestus.Release.V1.ReleaseChange do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ReleaseChange",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:event_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "eventId")
  field(:cursor, 2, type: Hephaestus.Common.V1.Cursor)
  field(:release_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:repository_id, 4, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:aggregate_version, 5, type: :uint64, json_name: "aggregateVersion")
  field(:change, 6, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 7, type: Hephaestus.Event.V1.LifecycleState, enum: true)
  field(:occurred_at, 8, type: Google.Protobuf.Timestamp, json_name: "occurredAt")
end

defmodule Hephaestus.Release.V1.WatchReleaseResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.WatchReleaseResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:item, 0)

  field(:sequence, 1, type: :uint64)
  field(:committed_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "committedCursor")

  field(:snapshot_barrier, 10,
    type: Hephaestus.Event.V1.ScopeSnapshotBarrier,
    json_name: "snapshotBarrier",
    oneof: 0
  )

  field(:event, 11, type: Hephaestus.Release.V1.ReleaseChange, oneof: 0)

  field(:retention_gap, 12,
    type: Hephaestus.Event.V1.RetentionGap,
    json_name: "retentionGap",
    oneof: 0
  )

  field(:access_revoked, 13,
    type: Hephaestus.Event.V1.AccessRevoked,
    json_name: "accessRevoked",
    oneof: 0
  )
end

defmodule Hephaestus.Release.V1.ReleaseService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.release.v1.ReleaseService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :ListRepositoryReleases,
    Hephaestus.Release.V1.ListRepositoryReleasesRequest,
    Hephaestus.Release.V1.ListRepositoryReleasesResponse
  )

  rpc(
    :GetRelease,
    Hephaestus.Release.V1.GetReleaseRequest,
    Hephaestus.Release.V1.GetReleaseResponse
  )

  rpc(
    :SetDraftVersion,
    Hephaestus.Release.V1.SetDraftVersionRequest,
    Hephaestus.Release.V1.SetDraftVersionResponse
  )

  rpc(
    :PublishRelease,
    Hephaestus.Release.V1.PublishReleaseRequest,
    Hephaestus.Release.V1.PublishReleaseResponse
  )

  rpc(
    :WatchRelease,
    Hephaestus.Release.V1.WatchReleaseRequest,
    stream(Hephaestus.Release.V1.WatchReleaseResponse)
  )
end

defmodule Hephaestus.Release.V1.ReleaseService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Release.V1.ReleaseService.Service
end
