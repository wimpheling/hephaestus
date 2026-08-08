defmodule Hephaestus.Project.V1.CreateProjectRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.CreateProjectRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:organization_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:name, 3, type: :string)
  field(:description, 4, type: :string)
end

defmodule Hephaestus.Project.V1.CreateProjectResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.CreateProjectResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
end

defmodule Hephaestus.Project.V1.Project do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.Project",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:name, 2, type: :string)
  field(:organization_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:organization_name, 4, type: :string, json_name: "organizationName")
  field(:description, 5, type: :string)
end

defmodule Hephaestus.Project.V1.ProjectRepository do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ProjectRepository",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:name, 2, type: :string)
  field(:default_branch, 3, type: :string, json_name: "defaultBranch")
  field(:is_public, 4, type: :bool, json_name: "isPublic")
  field(:attachment_count, 5, type: :int64, json_name: "attachmentCount")
  field(:run_count, 6, type: :int64, json_name: "runCount")
end

defmodule Hephaestus.Project.V1.InstanceSummary do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.InstanceSummary",
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

  field(:state_volume_id, 6,
    proto3_optional: true,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "stateVolumeId"
  )

  field(:updated_at, 7, type: Google.Protobuf.Timestamp, json_name: "updatedAt")
  field(:runnable, 8, type: :bool)
  field(:platform_policy_version, 9, type: :string, json_name: "platformPolicyVersion")
  field(:diagnostics, 10, repeated: true, type: Hephaestus.Common.V1.Diagnostic)
  field(:release_id, 11, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:release_version, 12, type: :string, json_name: "releaseVersion")
  field(:release_state, 13, type: :string, json_name: "releaseState")
  field(:release_agent_name, 14, type: :string, json_name: "releaseAgentName")
  field(:attachment_count, 15, type: :int64, json_name: "attachmentCount")
  field(:run_count, 16, type: :int64, json_name: "runCount")

  field(:last_run_at, 17,
    proto3_optional: true,
    type: Google.Protobuf.Timestamp,
    json_name: "lastRunAt"
  )
end

defmodule Hephaestus.Project.V1.ReleaseAgentOption do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ReleaseAgentOption",
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
  field(:release_id, 7, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:release_version, 8, type: :string, json_name: "releaseVersion")
  field(:source_commit, 9, type: :string, json_name: "sourceCommit")
  field(:repository_id, 10, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:repository_name, 11, type: :string, json_name: "repositoryName")

  field(:capability_requirements, 12,
    repeated: true,
    type: Hephaestus.Instance.V1.CapabilityRequirement,
    json_name: "capabilityRequirements"
  )
end

defmodule Hephaestus.Project.V1.GetProjectRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.GetProjectRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
end

defmodule Hephaestus.Project.V1.GetProjectResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.GetProjectResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project, 1, type: Hephaestus.Project.V1.Project)
end

defmodule Hephaestus.Project.V1.ListProjectRepositoriesRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ListProjectRepositoriesRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Project.V1.ListProjectRepositoriesResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ListProjectRepositoriesResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repositories, 1, repeated: true, type: Hephaestus.Project.V1.ProjectRepository)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Project.V1.ListProjectInstancesRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ListProjectInstancesRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Project.V1.ListProjectInstancesResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ListProjectInstancesResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:instances, 1, repeated: true, type: Hephaestus.Project.V1.InstanceSummary)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Project.V1.ListImportableReleaseAgentsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ListImportableReleaseAgentsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Project.V1.ListImportableReleaseAgentsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ListImportableReleaseAgentsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:release_agents, 1,
    repeated: true,
    type: Hephaestus.Project.V1.ReleaseAgentOption,
    json_name: "releaseAgents"
  )

  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Project.V1.ProjectService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.project.v1.ProjectService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :CreateProject,
    Hephaestus.Project.V1.CreateProjectRequest,
    Hephaestus.Project.V1.CreateProjectResponse
  )

  rpc(
    :GetProject,
    Hephaestus.Project.V1.GetProjectRequest,
    Hephaestus.Project.V1.GetProjectResponse
  )

  rpc(
    :ListProjectRepositories,
    Hephaestus.Project.V1.ListProjectRepositoriesRequest,
    Hephaestus.Project.V1.ListProjectRepositoriesResponse
  )

  rpc(
    :ListProjectInstances,
    Hephaestus.Project.V1.ListProjectInstancesRequest,
    Hephaestus.Project.V1.ListProjectInstancesResponse
  )

  rpc(
    :ListImportableReleaseAgents,
    Hephaestus.Project.V1.ListImportableReleaseAgentsRequest,
    Hephaestus.Project.V1.ListImportableReleaseAgentsResponse
  )
end

defmodule Hephaestus.Project.V1.ProjectService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Project.V1.ProjectService.Service
end
