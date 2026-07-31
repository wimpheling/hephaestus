defmodule Hephaestus.Event.V1.EventScopeKind do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.event.v1.EventScopeKind",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:EVENT_SCOPE_KIND_UNSPECIFIED, 0)
  field(:EVENT_SCOPE_KIND_IDENTITY, 1)
  field(:EVENT_SCOPE_KIND_ORGANIZATION, 2)
  field(:EVENT_SCOPE_KIND_PROJECT, 3)
  field(:EVENT_SCOPE_KIND_REPOSITORY, 4)
  field(:EVENT_SCOPE_KIND_RUN, 5)
  field(:EVENT_SCOPE_KIND_AGENT_INSTANCE, 6)
end

defmodule Hephaestus.Event.V1.AggregateType do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.event.v1.AggregateType",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:AGGREGATE_TYPE_UNSPECIFIED, 0)
  field(:AGGREGATE_TYPE_IDENTITY_ORGANIZATIONS, 1)
  field(:AGGREGATE_TYPE_ORGANIZATION, 2)
  field(:AGGREGATE_TYPE_PROJECT, 3)
  field(:AGGREGATE_TYPE_REPOSITORY, 4)
  field(:AGGREGATE_TYPE_REPOSITORY_REF, 5)
  field(:AGGREGATE_TYPE_BUILD, 6)
  field(:AGGREGATE_TYPE_RELEASE, 7)
  field(:AGGREGATE_TYPE_AGENT_INSTANCE, 8)
  field(:AGGREGATE_TYPE_RUN, 9)
  field(:AGGREGATE_TYPE_REVIEW, 10)
  field(:AGGREGATE_TYPE_SECRET_METADATA, 11)
  field(:AGGREGATE_TYPE_SECRET_GRANT, 12)
  field(:AGGREGATE_TYPE_SECRET_IMPORT, 13)
  field(:AGGREGATE_TYPE_AGENT_SECRET_BINDING, 14)
  field(:AGGREGATE_TYPE_ARTIFACT, 15)
  field(:AGGREGATE_TYPE_IDENTITY_PROFILE, 16)
end

defmodule Hephaestus.Event.V1.ChangeKind do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.event.v1.ChangeKind",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:CHANGE_KIND_UNSPECIFIED, 0)
  field(:CHANGE_KIND_CREATED, 1)
  field(:CHANGE_KIND_UPDATED, 2)
  field(:CHANGE_KIND_STATE_CHANGED, 3)
  field(:CHANGE_KIND_REMOVED, 4)
end

defmodule Hephaestus.Event.V1.LifecycleState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.event.v1.LifecycleState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:LIFECYCLE_STATE_UNSPECIFIED, 0)
  field(:LIFECYCLE_STATE_PENDING, 1)
  field(:LIFECYCLE_STATE_QUEUED, 2)
  field(:LIFECYCLE_STATE_RUNNING, 3)
  field(:LIFECYCLE_STATE_ACTIVE, 4)
  field(:LIFECYCLE_STATE_PAUSED, 5)
  field(:LIFECYCLE_STATE_SUCCEEDED, 6)
  field(:LIFECYCLE_STATE_FAILED, 7)
  field(:LIFECYCLE_STATE_PUBLISHED, 8)
  field(:LIFECYCLE_STATE_REVOKED, 9)
  field(:LIFECYCLE_STATE_DISABLED, 10)
  field(:LIFECYCLE_STATE_REJECTED, 11)
  field(:LIFECYCLE_STATE_CONFLICTED, 12)
  field(:LIFECYCLE_STATE_REMOVED, 13)
end

defmodule Hephaestus.Event.V1.EventScope do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.EventScope",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:kind, 1, type: Hephaestus.Event.V1.EventScopeKind, enum: true)
  field(:id, 2, type: Hephaestus.Common.V1.OpaqueId)
end

defmodule Hephaestus.Event.V1.EventProvenance do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.EventProvenance",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:actor_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "actorId")
  field(:request_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "requestId")
end

defmodule Hephaestus.Event.V1.IdentityOrganizationsChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.IdentityOrganizationsChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:organization_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:change, 2, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 3, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.IdentityProfileChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.IdentityProfileChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:change, 1, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 2, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.OrganizationChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.OrganizationChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:change, 1, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 2, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.ProjectChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.ProjectChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:organization_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:change, 2, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 3, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.RepositoryChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.RepositoryChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:change, 2, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 3, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.RepositoryRefChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.RepositoryRefChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:change, 1, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 2, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.BuildChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.BuildChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:change, 2, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 3, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.ReleaseChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.ReleaseChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:change, 2, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 3, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.AgentInstanceChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.AgentInstanceChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:change, 2, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 3, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.RunChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.RunChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:repository_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:change, 3, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 4, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.ReviewChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.ReviewChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:run_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "runId")
  field(:change, 2, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 3, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.SecretMetadataChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.SecretMetadataChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:owner_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "ownerId")
  field(:change, 2, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 3, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.SecretGrantChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.SecretGrantChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:secret_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
  field(:target_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "targetId")
  field(:change, 3, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 4, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.SecretImportChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.SecretImportChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:secret_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretId")
  field(:target_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "targetId")
  field(:change, 3, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 4, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.AgentSecretBindingChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.AgentSecretBindingChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:agent_instance_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "agentInstanceId")
  field(:secret_import_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "secretImportId")
  field(:change, 3, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 4, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.ArtifactChanged do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.ArtifactChanged",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:release_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:build_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildId")
  field(:change, 3, type: Hephaestus.Event.V1.ChangeKind, enum: true)
  field(:state, 4, type: Hephaestus.Event.V1.LifecycleState, enum: true)
end

defmodule Hephaestus.Event.V1.ProductEvent do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.ProductEvent",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:payload, 0)

  field(:event_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "eventId")
  field(:cursor, 2, type: Hephaestus.Common.V1.Cursor)
  field(:scope, 3, type: Hephaestus.Event.V1.EventScope)

  field(:aggregate_type, 4,
    type: Hephaestus.Event.V1.AggregateType,
    json_name: "aggregateType",
    enum: true
  )

  field(:aggregate_id, 5, type: Hephaestus.Common.V1.OpaqueId, json_name: "aggregateId")
  field(:aggregate_version, 6, type: :uint64, json_name: "aggregateVersion")
  field(:occurred_at, 7, type: Google.Protobuf.Timestamp, json_name: "occurredAt")
  field(:provenance, 8, type: Hephaestus.Event.V1.EventProvenance)
  field(:schema_version, 9, type: :uint32, json_name: "schemaVersion")

  field(:identity_organizations_changed, 20,
    type: Hephaestus.Event.V1.IdentityOrganizationsChanged,
    json_name: "identityOrganizationsChanged",
    oneof: 0
  )

  field(:organization_changed, 21,
    type: Hephaestus.Event.V1.OrganizationChanged,
    json_name: "organizationChanged",
    oneof: 0
  )

  field(:project_changed, 22,
    type: Hephaestus.Event.V1.ProjectChanged,
    json_name: "projectChanged",
    oneof: 0
  )

  field(:repository_changed, 23,
    type: Hephaestus.Event.V1.RepositoryChanged,
    json_name: "repositoryChanged",
    oneof: 0
  )

  field(:repository_ref_changed, 24,
    type: Hephaestus.Event.V1.RepositoryRefChanged,
    json_name: "repositoryRefChanged",
    oneof: 0
  )

  field(:build_changed, 25,
    type: Hephaestus.Event.V1.BuildChanged,
    json_name: "buildChanged",
    oneof: 0
  )

  field(:release_changed, 26,
    type: Hephaestus.Event.V1.ReleaseChanged,
    json_name: "releaseChanged",
    oneof: 0
  )

  field(:agent_instance_changed, 27,
    type: Hephaestus.Event.V1.AgentInstanceChanged,
    json_name: "agentInstanceChanged",
    oneof: 0
  )

  field(:run_changed, 28, type: Hephaestus.Event.V1.RunChanged, json_name: "runChanged", oneof: 0)

  field(:review_changed, 29,
    type: Hephaestus.Event.V1.ReviewChanged,
    json_name: "reviewChanged",
    oneof: 0
  )

  field(:secret_metadata_changed, 30,
    type: Hephaestus.Event.V1.SecretMetadataChanged,
    json_name: "secretMetadataChanged",
    oneof: 0
  )

  field(:secret_grant_changed, 31,
    type: Hephaestus.Event.V1.SecretGrantChanged,
    json_name: "secretGrantChanged",
    oneof: 0
  )

  field(:secret_import_changed, 32,
    type: Hephaestus.Event.V1.SecretImportChanged,
    json_name: "secretImportChanged",
    oneof: 0
  )

  field(:agent_secret_binding_changed, 33,
    type: Hephaestus.Event.V1.AgentSecretBindingChanged,
    json_name: "agentSecretBindingChanged",
    oneof: 0
  )

  field(:artifact_changed, 34,
    type: Hephaestus.Event.V1.ArtifactChanged,
    json_name: "artifactChanged",
    oneof: 0
  )

  field(:identity_profile_changed, 35,
    type: Hephaestus.Event.V1.IdentityProfileChanged,
    json_name: "identityProfileChanged",
    oneof: 0
  )
end

defmodule Hephaestus.Event.V1.AggregateVersionReference do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.AggregateVersionReference",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:aggregate_type, 1,
    type: Hephaestus.Event.V1.AggregateType,
    json_name: "aggregateType",
    enum: true
  )

  field(:aggregate_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "aggregateId")
  field(:aggregate_version, 3, type: :uint64, json_name: "aggregateVersion")
end

defmodule Hephaestus.Event.V1.ScopeSnapshotBarrier do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.ScopeSnapshotBarrier",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:scope, 1, type: Hephaestus.Event.V1.EventScope)
  field(:committed_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "committedCursor")

  field(:aggregate_versions, 3,
    repeated: true,
    type: Hephaestus.Event.V1.AggregateVersionReference,
    json_name: "aggregateVersions"
  )

  field(:schema_version, 4, type: :uint32, json_name: "schemaVersion")
end

defmodule Hephaestus.Event.V1.RetentionGap do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.RetentionGap",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:scope, 1, type: Hephaestus.Event.V1.EventScope)
  field(:requested_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "requestedCursor")

  field(:earliest_available_cursor, 3,
    type: Hephaestus.Common.V1.Cursor,
    json_name: "earliestAvailableCursor"
  )

  field(:latest_committed_cursor, 4,
    type: Hephaestus.Common.V1.Cursor,
    json_name: "latestCommittedCursor"
  )
end

defmodule Hephaestus.Event.V1.AccessRevoked do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.AccessRevoked",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:scope, 1, type: Hephaestus.Event.V1.EventScope)
  field(:observed_at, 2, type: Google.Protobuf.Timestamp, json_name: "observedAt")
end

defmodule Hephaestus.Event.V1.WatchIdentityRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchIdentityRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:resume_cursor, 1, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_events, 2, type: :uint32, json_name: "maxEvents")
  field(:max_total_bytes, 3, type: :uint64, json_name: "maxTotalBytes")
end

defmodule Hephaestus.Event.V1.WatchOrganizationRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchOrganizationRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:organization_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:resume_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_events, 3, type: :uint32, json_name: "maxEvents")
  field(:max_total_bytes, 4, type: :uint64, json_name: "maxTotalBytes")
end

defmodule Hephaestus.Event.V1.WatchProjectRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchProjectRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:resume_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_events, 3, type: :uint32, json_name: "maxEvents")
  field(:max_total_bytes, 4, type: :uint64, json_name: "maxTotalBytes")
end

defmodule Hephaestus.Event.V1.WatchRepositoryRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchRepositoryRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:resume_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_events, 3, type: :uint32, json_name: "maxEvents")
  field(:max_total_bytes, 4, type: :uint64, json_name: "maxTotalBytes")
end

defmodule Hephaestus.Event.V1.WatchRunRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchRunRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:run_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "runId")
  field(:resume_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_events, 3, type: :uint32, json_name: "maxEvents")
  field(:max_total_bytes, 4, type: :uint64, json_name: "maxTotalBytes")
end

defmodule Hephaestus.Event.V1.WatchAgentInstanceRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchAgentInstanceRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:agent_instance_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "agentInstanceId")
  field(:resume_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_events, 3, type: :uint32, json_name: "maxEvents")
  field(:max_total_bytes, 4, type: :uint64, json_name: "maxTotalBytes")
end

defmodule Hephaestus.Event.V1.WatchIdentityResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchIdentityResponse",
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

  field(:event, 11, type: Hephaestus.Event.V1.ProductEvent, oneof: 0)

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

defmodule Hephaestus.Event.V1.WatchOrganizationResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchOrganizationResponse",
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

  field(:event, 11, type: Hephaestus.Event.V1.ProductEvent, oneof: 0)

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

defmodule Hephaestus.Event.V1.WatchProjectResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchProjectResponse",
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

  field(:event, 11, type: Hephaestus.Event.V1.ProductEvent, oneof: 0)

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

defmodule Hephaestus.Event.V1.WatchRepositoryResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchRepositoryResponse",
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

  field(:event, 11, type: Hephaestus.Event.V1.ProductEvent, oneof: 0)

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

defmodule Hephaestus.Event.V1.WatchRunResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchRunResponse",
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

  field(:event, 11, type: Hephaestus.Event.V1.ProductEvent, oneof: 0)

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

defmodule Hephaestus.Event.V1.WatchAgentInstanceResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.event.v1.WatchAgentInstanceResponse",
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

  field(:event, 11, type: Hephaestus.Event.V1.ProductEvent, oneof: 0)

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

defmodule Hephaestus.Event.V1.ProductEventService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.event.v1.ProductEventService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :WatchIdentity,
    Hephaestus.Event.V1.WatchIdentityRequest,
    stream(Hephaestus.Event.V1.WatchIdentityResponse)
  )

  rpc(
    :WatchOrganization,
    Hephaestus.Event.V1.WatchOrganizationRequest,
    stream(Hephaestus.Event.V1.WatchOrganizationResponse)
  )

  rpc(
    :WatchProject,
    Hephaestus.Event.V1.WatchProjectRequest,
    stream(Hephaestus.Event.V1.WatchProjectResponse)
  )

  rpc(
    :WatchRepository,
    Hephaestus.Event.V1.WatchRepositoryRequest,
    stream(Hephaestus.Event.V1.WatchRepositoryResponse)
  )

  rpc(
    :WatchRun,
    Hephaestus.Event.V1.WatchRunRequest,
    stream(Hephaestus.Event.V1.WatchRunResponse)
  )

  rpc(
    :WatchAgentInstance,
    Hephaestus.Event.V1.WatchAgentInstanceRequest,
    stream(Hephaestus.Event.V1.WatchAgentInstanceResponse)
  )
end

defmodule Hephaestus.Event.V1.ProductEventService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Event.V1.ProductEventService.Service
end
