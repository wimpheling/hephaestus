defmodule Hephaestus.Repository.V1.RepositoryRun do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository.v1.RepositoryRun",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:state, 2, type: :string)
  field(:outcome, 3, type: :string)
  field(:exit_code, 4, proto3_optional: true, type: :int32, json_name: "exitCode")
  field(:failure, 5, repeated: true, type: Hephaestus.Common.V1.Diagnostic)
  field(:created_at, 6, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:updated_at, 7, type: Google.Protobuf.Timestamp, json_name: "updatedAt")
  field(:agent_name, 8, type: :string, json_name: "agentName")
  field(:commit_sha, 9, type: :string, json_name: "commitSha")
  field(:git_ref, 10, type: :string, json_name: "gitRef")
  field(:attempt, 11, type: :uint32)
  field(:proposal_id, 12, type: Hephaestus.Common.V1.OpaqueId, json_name: "proposalId")
  field(:proposal_state, 13, type: :string, json_name: "proposalState")
end

defmodule Hephaestus.Repository.V1.Repository do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository.v1.Repository",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:name, 2, type: :string)
  field(:default_branch, 3, type: :string, json_name: "defaultBranch")
  field(:is_public, 4, type: :bool, json_name: "isPublic")
  field(:project_id, 5, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:project_name, 6, type: :string, json_name: "projectName")
  field(:organization_id, 7, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:organization_name, 8, type: :string, json_name: "organizationName")
  field(:runs, 9, repeated: true, type: Hephaestus.Repository.V1.RepositoryRun)
end

defmodule Hephaestus.Repository.V1.RepositoryInstanceAttachment do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository.v1.RepositoryInstanceAttachment",
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
  field(:instance_id, 6, type: Hephaestus.Common.V1.OpaqueId, json_name: "instanceId")
  field(:instance_name, 7, type: :string, json_name: "instanceName")
  field(:instance_state, 8, type: :string, json_name: "instanceState")
  field(:project_id, 9, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:project_name, 10, type: :string, json_name: "projectName")
  field(:release_id, 11, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:release_version, 12, type: :string, json_name: "releaseVersion")
end

defmodule Hephaestus.Repository.V1.GetRepositoryRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository.v1.GetRepositoryRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
end

defmodule Hephaestus.Repository.V1.GetRepositoryResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository.v1.GetRepositoryResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository, 1, type: Hephaestus.Repository.V1.Repository)
end

defmodule Hephaestus.Repository.V1.ListRepositoryInstancesRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository.v1.ListRepositoryInstancesRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Repository.V1.ListRepositoryInstancesResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository.v1.ListRepositoryInstancesResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:attachments, 1,
    repeated: true,
    type: Hephaestus.Repository.V1.RepositoryInstanceAttachment
  )

  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Repository.V1.RepositoryService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.repository.v1.RepositoryService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :GetRepository,
    Hephaestus.Repository.V1.GetRepositoryRequest,
    Hephaestus.Repository.V1.GetRepositoryResponse
  )

  rpc(
    :ListRepositoryInstances,
    Hephaestus.Repository.V1.ListRepositoryInstancesRequest,
    Hephaestus.Repository.V1.ListRepositoryInstancesResponse
  )
end

defmodule Hephaestus.Repository.V1.RepositoryService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Repository.V1.RepositoryService.Service
end
