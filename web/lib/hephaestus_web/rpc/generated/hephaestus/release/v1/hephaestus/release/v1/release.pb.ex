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

defmodule Hephaestus.Release.V1.ReleaseUiScope do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.release.v1.ReleaseUiScope",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RELEASE_UI_SCOPE_UNSPECIFIED, 0)
  field(:RELEASE_UI_SCOPE_PROJECT, 1)
  field(:RELEASE_UI_SCOPE_REPOSITORY, 2)
  field(:RELEASE_UI_SCOPE_GLOBAL, 3)
end

defmodule Hephaestus.Release.V1.ReleaseUiIcon do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.release.v1.ReleaseUiIcon",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RELEASE_UI_ICON_UNSPECIFIED, 0)
  field(:RELEASE_UI_ICON_APP, 1)
  field(:RELEASE_UI_ICON_CHAT, 2)
  field(:RELEASE_UI_ICON_CODE, 3)
  field(:RELEASE_UI_ICON_BOOK, 4)
  field(:RELEASE_UI_ICON_CHART, 5)
end

defmodule Hephaestus.Release.V1.ReleaseUiPresentation do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.release.v1.ReleaseUiPresentation",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RELEASE_UI_PRESENTATION_UNSPECIFIED, 0)
  field(:RELEASE_UI_PRESENTATION_IFRAME, 1)
  field(:RELEASE_UI_PRESENTATION_FULL_PAGE, 2)
end

defmodule Hephaestus.Release.V1.ReleaseUiCachePolicy do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.release.v1.ReleaseUiCachePolicy",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RELEASE_UI_CACHE_POLICY_UNSPECIFIED, 0)
  field(:RELEASE_UI_CACHE_POLICY_NO_STORE, 1)
end

defmodule Hephaestus.Release.V1.ReleaseUiRepositoryGitAccess do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.release.v1.ReleaseUiRepositoryGitAccess",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RELEASE_UI_REPOSITORY_GIT_ACCESS_UNSPECIFIED, 0)
  field(:RELEASE_UI_REPOSITORY_GIT_ACCESS_NONE, 1)
  field(:RELEASE_UI_REPOSITORY_GIT_ACCESS_READ, 2)
  field(:RELEASE_UI_REPOSITORY_GIT_ACCESS_READ_WRITE, 3)
end

defmodule Hephaestus.Release.V1.UiInstallationLifecycle do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.release.v1.UiInstallationLifecycle",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:UI_INSTALLATION_LIFECYCLE_UNSPECIFIED, 0)
  field(:UI_INSTALLATION_LIFECYCLE_ENABLED, 1)
  field(:UI_INSTALLATION_LIFECYCLE_DISABLED, 2)
  field(:UI_INSTALLATION_LIFECYCLE_REMOVED, 3)
end

defmodule Hephaestus.Release.V1.UiInstallationContentKind do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.release.v1.UiInstallationContentKind",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:UI_INSTALLATION_CONTENT_KIND_UNSPECIFIED, 0)
  field(:UI_INSTALLATION_CONTENT_KIND_STATIC, 1)
  field(:UI_INSTALLATION_CONTENT_KIND_MANAGED_SERVICE, 2)
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

  field(:ui_descriptors, 22,
    repeated: true,
    type: Hephaestus.Release.V1.ReleaseUiDescriptor,
    json_name: "uiDescriptors"
  )
end

defmodule Hephaestus.Release.V1.ReleaseUiDescriptor do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ReleaseUiDescriptor",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:content, 0)

  field(:key, 1, type: :string)
  field(:scope, 2, type: Hephaestus.Release.V1.ReleaseUiScope, enum: true)
  field(:label, 3, type: :string)
  field(:icon, 4, type: Hephaestus.Release.V1.ReleaseUiIcon, enum: true)
  field(:presentation, 5, type: Hephaestus.Release.V1.ReleaseUiPresentation, enum: true)
  field(:route_base, 6, type: :string, json_name: "routeBase")
  field(:entrypoint, 7, type: :string)
  field(:ui_kit_version, 8, type: :uint32, json_name: "uiKitVersion")
  field(:cache, 9, type: Hephaestus.Release.V1.ReleaseUiCachePolicy, enum: true)

  field(:static_content, 10,
    type: Hephaestus.Release.V1.ReleaseUiStaticContent,
    json_name: "staticContent",
    oneof: 0
  )

  field(:managed_service, 11,
    type: Hephaestus.Release.V1.ReleaseUiManagedService,
    json_name: "managedService",
    oneof: 0
  )

  field(:apis, 12, repeated: true, type: Hephaestus.Release.V1.ReleaseUiApiBinding)

  field(:repository_git_access, 13,
    type: Hephaestus.Release.V1.ReleaseUiRepositoryGitAccess,
    json_name: "repositoryGitAccess",
    enum: true
  )
end

defmodule Hephaestus.Release.V1.ReleaseUiStaticContent do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ReleaseUiStaticContent",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:files, 1, repeated: true, type: Hephaestus.Release.V1.ReleaseUiStaticFile)
end

defmodule Hephaestus.Release.V1.ReleaseUiStaticFile do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ReleaseUiStaticFile",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:route, 1, type: :string)
  field(:artifact_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "artifactId")
  field(:media_type, 3, type: :string, json_name: "mediaType")
end

defmodule Hephaestus.Release.V1.ReleaseUiManagedService do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ReleaseUiManagedService",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:gateway_name, 1, type: :string, json_name: "gatewayName")
  field(:route, 2, type: :string)
  field(:release_agent_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseAgentId")
end

defmodule Hephaestus.Release.V1.ReleaseUiApiBinding do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ReleaseUiApiBinding",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:key, 1, type: :string)
  field(:gateway_name, 2, type: :string, json_name: "gatewayName")
  field(:method, 3, type: :string)
  field(:route, 4, type: :string)
  field(:release_agent_id, 5, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseAgentId")
end

defmodule Hephaestus.Release.V1.UiInstallationTarget do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.UiInstallationTarget",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  oneof(:target, 0)

  field(:global, 1, type: Hephaestus.Release.V1.GlobalUiInstallationTarget, oneof: 0)
  field(:project_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId", oneof: 0)

  field(:repository_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "repositoryId",
    oneof: 0
  )
end

defmodule Hephaestus.Release.V1.GlobalUiInstallationTarget do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.GlobalUiInstallationTarget",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3
end

defmodule Hephaestus.Release.V1.InstallUiRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.InstallUiRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:organization_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:target, 3, type: Hephaestus.Release.V1.UiInstallationTarget)
  field(:release_id, 4, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:ui_key, 5, type: :string, json_name: "uiKey")

  field(:acknowledge_repository_git_access, 6,
    type: :bool,
    json_name: "acknowledgeRepositoryGitAccess"
  )
end

defmodule Hephaestus.Release.V1.InstallUiResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.InstallUiResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:installation_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")
  field(:generation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "generationId")
  field(:lifecycle, 3, type: Hephaestus.Release.V1.UiInstallationLifecycle, enum: true)
  field(:receipt, 4, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Release.V1.ActivateUiRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ActivateUiRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:installation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")

  field(:expected_generation_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "expectedGenerationId"
  )

  field(:release_id, 4, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:ui_key, 5, type: :string, json_name: "uiKey")
end

defmodule Hephaestus.Release.V1.ActivateUiResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ActivateUiResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:installation_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")
  field(:generation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "generationId")
  field(:lifecycle, 3, type: Hephaestus.Release.V1.UiInstallationLifecycle, enum: true)
  field(:receipt, 4, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Release.V1.RollbackUiRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.RollbackUiRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:installation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")

  field(:expected_generation_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "expectedGenerationId"
  )

  field(:release_id, 4, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:ui_key, 5, type: :string, json_name: "uiKey")
end

defmodule Hephaestus.Release.V1.RollbackUiResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.RollbackUiResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:installation_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")
  field(:generation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "generationId")
  field(:lifecycle, 3, type: Hephaestus.Release.V1.UiInstallationLifecycle, enum: true)
  field(:receipt, 4, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Release.V1.DisableUiRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.DisableUiRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:installation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")

  field(:expected_generation_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "expectedGenerationId"
  )
end

defmodule Hephaestus.Release.V1.DisableUiResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.DisableUiResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:installation_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")
  field(:generation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "generationId")
  field(:lifecycle, 3, type: Hephaestus.Release.V1.UiInstallationLifecycle, enum: true)
  field(:receipt, 4, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Release.V1.RemoveUiRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.RemoveUiRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:installation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")

  field(:expected_generation_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "expectedGenerationId"
  )
end

defmodule Hephaestus.Release.V1.RemoveUiResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.RemoveUiResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:installation_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")
  field(:generation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "generationId")
  field(:lifecycle, 3, type: Hephaestus.Release.V1.UiInstallationLifecycle, enum: true)
  field(:receipt, 4, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Release.V1.ListUiInstallationsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ListUiInstallationsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:organization_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:target, 2, type: Hephaestus.Release.V1.UiInstallationTarget)
  field(:page, 3, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Release.V1.ListUiInstallationsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.ListUiInstallationsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:installations, 1, repeated: true, type: Hephaestus.Release.V1.UiInstallationNavigation)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Release.V1.UiInstallationNavigation do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.UiInstallationNavigation",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:installation_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")
  field(:generation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "generationId")
  field(:organization_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:target, 4, type: Hephaestus.Release.V1.UiInstallationTarget)
  field(:lifecycle, 5, type: Hephaestus.Release.V1.UiInstallationLifecycle, enum: true)
  field(:release_id, 6, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:ui_key, 7, type: :string, json_name: "uiKey")
  field(:label, 8, type: :string)
  field(:icon, 9, type: Hephaestus.Release.V1.ReleaseUiIcon, enum: true)
  field(:presentation, 10, type: Hephaestus.Release.V1.ReleaseUiPresentation, enum: true)
  field(:route_base, 11, type: :string, json_name: "routeBase")

  field(:content_kind, 12,
    type: Hephaestus.Release.V1.UiInstallationContentKind,
    json_name: "contentKind",
    enum: true
  )

  field(:launchable, 13, type: :bool)
end

defmodule Hephaestus.Release.V1.CreateUiBrowserHandoffRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.CreateUiBrowserHandoffRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:installation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")
  field(:generation_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "generationId")
  field(:route, 4, type: :string)
  field(:handoff_secret, 5, type: :bytes, json_name: "handoffSecret", deprecated: false)
end

defmodule Hephaestus.Release.V1.CreateUiBrowserHandoffResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.release.v1.CreateUiBrowserHandoffResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:handoff_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "handoffId")
  field(:installation_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "installationId")
  field(:generation_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "generationId")
  field(:route, 4, type: :string)
  field(:expires_at, 5, type: Google.Protobuf.Timestamp, json_name: "expiresAt")
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

  rpc(:InstallUi, Hephaestus.Release.V1.InstallUiRequest, Hephaestus.Release.V1.InstallUiResponse)

  rpc(
    :ActivateUi,
    Hephaestus.Release.V1.ActivateUiRequest,
    Hephaestus.Release.V1.ActivateUiResponse
  )

  rpc(
    :RollbackUi,
    Hephaestus.Release.V1.RollbackUiRequest,
    Hephaestus.Release.V1.RollbackUiResponse
  )

  rpc(:DisableUi, Hephaestus.Release.V1.DisableUiRequest, Hephaestus.Release.V1.DisableUiResponse)

  rpc(:RemoveUi, Hephaestus.Release.V1.RemoveUiRequest, Hephaestus.Release.V1.RemoveUiResponse)

  rpc(
    :ListUiInstallations,
    Hephaestus.Release.V1.ListUiInstallationsRequest,
    Hephaestus.Release.V1.ListUiInstallationsResponse
  )

  rpc(
    :CreateUiBrowserHandoff,
    Hephaestus.Release.V1.CreateUiBrowserHandoffRequest,
    Hephaestus.Release.V1.CreateUiBrowserHandoffResponse
  )
end

defmodule Hephaestus.Release.V1.ReleaseService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Release.V1.ReleaseService.Service
end
