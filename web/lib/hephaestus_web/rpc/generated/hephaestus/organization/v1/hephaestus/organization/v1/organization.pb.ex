defmodule Hephaestus.Organization.V1.OrganizationSummary do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.OrganizationSummary",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:name, 2, type: :string)
  field(:project_count, 3, type: :int64, json_name: "projectCount")
  field(:repository_count, 4, type: :int64, json_name: "repositoryCount")
end

defmodule Hephaestus.Organization.V1.Organization do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.Organization",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:name, 2, type: :string)
end

defmodule Hephaestus.Organization.V1.RepositorySummary do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.RepositorySummary",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:name, 2, type: :string)
  field(:default_branch, 3, type: :string, json_name: "defaultBranch")
  field(:is_public, 4, type: :bool, json_name: "isPublic")
  field(:project_name, 5, type: :string, json_name: "projectName")
  field(:run_count, 6, type: :int64, json_name: "runCount")

  field(:last_run_at, 7,
    proto3_optional: true,
    type: Google.Protobuf.Timestamp,
    json_name: "lastRunAt"
  )
end

defmodule Hephaestus.Organization.V1.ProjectSummary do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.ProjectSummary",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:name, 2, type: :string)
  field(:repository_count, 3, type: :int64, json_name: "repositoryCount")
  field(:instance_count, 4, type: :int64, json_name: "instanceCount")
  field(:run_count, 5, type: :int64, json_name: "runCount")

  field(:last_activity_at, 6,
    proto3_optional: true,
    type: Google.Protobuf.Timestamp,
    json_name: "lastActivityAt"
  )
end

defmodule Hephaestus.Organization.V1.ListOrganizationsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.ListOrganizationsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:page, 1, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Organization.V1.ListOrganizationsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.ListOrganizationsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:organizations, 1, repeated: true, type: Hephaestus.Organization.V1.OrganizationSummary)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Organization.V1.GetOrganizationRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.GetOrganizationRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:organization_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
end

defmodule Hephaestus.Organization.V1.GetOrganizationResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.GetOrganizationResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:organization, 1, type: Hephaestus.Organization.V1.Organization)
end

defmodule Hephaestus.Organization.V1.ListOrganizationRepositoriesRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.ListOrganizationRepositoriesRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:organization_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Organization.V1.ListOrganizationRepositoriesResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.ListOrganizationRepositoriesResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repositories, 1, repeated: true, type: Hephaestus.Organization.V1.RepositorySummary)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Organization.V1.ListOrganizationProjectsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.ListOrganizationProjectsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:organization_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "organizationId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Organization.V1.ListOrganizationProjectsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.organization.v1.ListOrganizationProjectsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:projects, 1, repeated: true, type: Hephaestus.Organization.V1.ProjectSummary)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Organization.V1.OrganizationService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.organization.v1.OrganizationService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :ListOrganizations,
    Hephaestus.Organization.V1.ListOrganizationsRequest,
    Hephaestus.Organization.V1.ListOrganizationsResponse
  )

  rpc(
    :GetOrganization,
    Hephaestus.Organization.V1.GetOrganizationRequest,
    Hephaestus.Organization.V1.GetOrganizationResponse
  )

  rpc(
    :ListOrganizationRepositories,
    Hephaestus.Organization.V1.ListOrganizationRepositoriesRequest,
    Hephaestus.Organization.V1.ListOrganizationRepositoriesResponse
  )

  rpc(
    :ListOrganizationProjects,
    Hephaestus.Organization.V1.ListOrganizationProjectsRequest,
    Hephaestus.Organization.V1.ListOrganizationProjectsResponse
  )
end

defmodule Hephaestus.Organization.V1.OrganizationService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Organization.V1.OrganizationService.Service
end
