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
  field(:source_commit, 9, type: :string, json_name: "sourceCommit")
  field(:source_ref, 10, type: :string, json_name: "sourceRef")
  field(:build_definition_hash, 11, type: :string, json_name: "buildDefinitionHash")
  field(:repository_id, 12, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:release_id, 13, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:release_state, 14, type: :string, json_name: "releaseState")
  field(:artifact_count, 15, type: :uint32, json_name: "artifactCount")
  field(:trigger, 16, type: :string)
  field(:agent_key, 17, type: :string, json_name: "agentKey")
  field(:builder_image_id, 18, type: Hephaestus.Common.V1.OpaqueId, json_name: "builderImageId")
  field(:builder_image_key, 19, type: :string, json_name: "builderImageKey")
  field(:builder_image_reference, 20, type: :string, json_name: "builderImageReference")
  field(:configuration_hash, 21, type: :string, json_name: "configurationHash")
  field(:parsed_declaration_json, 22, type: :string, json_name: "parsedDeclarationJson")
  field(:build_policy_json, 23, type: :string, json_name: "buildPolicyJson")
  field(:started_at, 24, type: Google.Protobuf.Timestamp, json_name: "startedAt")
  field(:completed_at, 25, type: Google.Protobuf.Timestamp, json_name: "completedAt")
  field(:duration_milliseconds, 26, type: :int64, json_name: "durationMilliseconds")
  field(:timeline, 27, repeated: true, type: Hephaestus.Build.V1.BuildTimelineEntry)

  field(:declared_artifacts, 28,
    repeated: true,
    type: Hephaestus.Build.V1.DeclaredArtifact,
    json_name: "declaredArtifacts"
  )

  field(:produced_artifacts, 29,
    repeated: true,
    type: Hephaestus.Build.V1.ProducedArtifact,
    json_name: "producedArtifacts"
  )

  field(:artifact_manifest_json, 30, type: :string, json_name: "artifactManifestJson")
  field(:release_version, 31, type: :string, json_name: "releaseVersion")
end

defmodule Hephaestus.Build.V1.BuildTimelineEntry do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.BuildTimelineEntry",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:from_state, 1, type: :string, json_name: "fromState")
  field(:to_state, 2, type: :string, json_name: "toState")
  field(:reason, 3, type: :string)
  field(:occurred_at, 4, type: Google.Protobuf.Timestamp, json_name: "occurredAt")
end

defmodule Hephaestus.Build.V1.DeclaredArtifact do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.DeclaredArtifact",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:path, 1, type: :string)
  field(:kind, 2, type: :string)
  field(:media_type, 3, type: :string, json_name: "mediaType")
end

defmodule Hephaestus.Build.V1.ProducedArtifact do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.ProducedArtifact",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:path, 1, type: :string)
  field(:kind, 2, type: :string)
  field(:mode, 3, type: :uint32)
  field(:sha256, 4, type: :string)
  field(:size_bytes, 5, type: :uint64, json_name: "sizeBytes")
  field(:media_type, 6, type: :string, json_name: "mediaType")
end

defmodule Hephaestus.Build.V1.ListBuildsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.ListBuildsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Build.V1.ListBuildsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.ListBuildsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:builds, 1, repeated: true, type: Hephaestus.Build.V1.Build)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
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

defmodule Hephaestus.Build.V1.RetryBuildRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.RetryBuildRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:build_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildId")
end

defmodule Hephaestus.Build.V1.RetryBuildResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.RetryBuildResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:build_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildId")
  field(:operation, 2, type: Hephaestus.Common.V1.Operation)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Build.V1.RebuildForVerificationRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.RebuildForVerificationRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:build_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildId")
end

defmodule Hephaestus.Build.V1.RebuildForVerificationResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.RebuildForVerificationResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:build_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildId")
  field(:operation, 2, type: Hephaestus.Common.V1.Operation)
  field(:receipt, 3, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Build.V1.WatchBuildRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.WatchBuildRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:build_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildId")
  field(:resume_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_events, 3, type: :uint32, json_name: "maxEvents")
  field(:max_total_bytes, 4, type: :uint64, json_name: "maxTotalBytes")
end

defmodule Hephaestus.Build.V1.WatchBuildResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.WatchBuildResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:item, 0)

  field(:sequence, 1, type: :uint64)
  field(:committed_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "committedCursor")
  field(:build, 3, type: Hephaestus.Build.V1.Build, oneof: 0)
  field(:heartbeat, 4, type: :bool, oneof: 0)
  field(:terminal, 5, type: :bool, oneof: 0)
end

defmodule Hephaestus.Build.V1.StreamBuildLogsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.StreamBuildLogsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:build_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildId")
  field(:resume_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_events, 3, type: :uint32, json_name: "maxEvents")
  field(:max_total_bytes, 4, type: :uint64, json_name: "maxTotalBytes")
  field(:max_chunk_bytes, 5, type: :uint64, json_name: "maxChunkBytes")
end

defmodule Hephaestus.Build.V1.StreamBuildLogsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.StreamBuildLogsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:sequence, 1, type: :uint64)
  field(:committed_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "committedCursor")
  field(:contents, 3, type: :string)
  field(:heartbeat, 4, type: :bool)
  field(:end_of_stream, 5, type: :bool, json_name: "endOfStream")
  field(:truncated, 6, type: :bool)
end

defmodule Hephaestus.Build.V1.WatchRepositoryBuildsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.WatchRepositoryBuildsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:resume_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_events, 3, type: :uint32, json_name: "maxEvents")
  field(:max_total_bytes, 4, type: :uint64, json_name: "maxTotalBytes")
end

defmodule Hephaestus.Build.V1.BuildChange do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.BuildChange",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:event_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "eventId")
  field(:cursor, 2, type: Hephaestus.Common.V1.Cursor)
  field(:build_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildId")
  field(:repository_id, 4, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:aggregate_version, 5, type: :uint64, json_name: "aggregateVersion")
  field(:change, 6, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 7, type: Hephaestus.Event.V1.LifecycleState, enum: true)
  field(:occurred_at, 8, type: Google.Protobuf.Timestamp, json_name: "occurredAt")
end

defmodule Hephaestus.Build.V1.WatchRepositoryBuildsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.build.v1.WatchRepositoryBuildsResponse",
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

  field(:event, 11, type: Hephaestus.Build.V1.BuildChange, oneof: 0)

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

defmodule Hephaestus.Build.V1.BuildService.Service do
  @moduledoc false

  use GRPC.Service, name: "hephaestus.build.v1.BuildService", protoc_gen_elixir_version: "0.17.0"

  rpc(:ListBuilds, Hephaestus.Build.V1.ListBuildsRequest, Hephaestus.Build.V1.ListBuildsResponse)

  rpc(:GetBuild, Hephaestus.Build.V1.GetBuildRequest, Hephaestus.Build.V1.GetBuildResponse)

  rpc(
    :RequestBuild,
    Hephaestus.Build.V1.RequestBuildRequest,
    Hephaestus.Build.V1.RequestBuildResponse
  )

  rpc(:RetryBuild, Hephaestus.Build.V1.RetryBuildRequest, Hephaestus.Build.V1.RetryBuildResponse)

  rpc(
    :RebuildForVerification,
    Hephaestus.Build.V1.RebuildForVerificationRequest,
    Hephaestus.Build.V1.RebuildForVerificationResponse
  )

  rpc(
    :WatchBuild,
    Hephaestus.Build.V1.WatchBuildRequest,
    stream(Hephaestus.Build.V1.WatchBuildResponse)
  )

  rpc(
    :WatchRepositoryBuilds,
    Hephaestus.Build.V1.WatchRepositoryBuildsRequest,
    stream(Hephaestus.Build.V1.WatchRepositoryBuildsResponse)
  )

  rpc(
    :StreamBuildLogs,
    Hephaestus.Build.V1.StreamBuildLogsRequest,
    stream(Hephaestus.Build.V1.StreamBuildLogsResponse)
  )
end

defmodule Hephaestus.Build.V1.BuildService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Build.V1.BuildService.Service
end
