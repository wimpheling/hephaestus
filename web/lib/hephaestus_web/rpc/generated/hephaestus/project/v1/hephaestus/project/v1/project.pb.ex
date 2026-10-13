defmodule Hephaestus.Project.V1.RecipeDeploymentLifecycle do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.project.v1.RecipeDeploymentLifecycle",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RECIPE_DEPLOYMENT_LIFECYCLE_UNSPECIFIED, 0)
  field(:RECIPE_DEPLOYMENT_LIFECYCLE_INSTALLING, 1)
  field(:RECIPE_DEPLOYMENT_LIFECYCLE_INSTALLED, 2)
  field(:RECIPE_DEPLOYMENT_LIFECYCLE_REMOVING, 3)
  field(:RECIPE_DEPLOYMENT_LIFECYCLE_REMOVED, 4)
  field(:RECIPE_DEPLOYMENT_LIFECYCLE_RECOVERY_REQUIRED, 5)
end

defmodule Hephaestus.Project.V1.RecipeResourceKind do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.project.v1.RecipeResourceKind",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RECIPE_RESOURCE_KIND_UNSPECIFIED, 0)
  field(:RECIPE_RESOURCE_KIND_VOLUME, 1)
  field(:RECIPE_RESOURCE_KIND_INSTANCE, 2)
end

defmodule Hephaestus.Project.V1.RecipeResourceOwnership do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.project.v1.RecipeResourceOwnership",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RECIPE_RESOURCE_OWNERSHIP_UNSPECIFIED, 0)
  field(:RECIPE_RESOURCE_OWNERSHIP_OWNED, 1)
  field(:RECIPE_RESOURCE_OWNERSHIP_EXTERNAL, 2)
end

defmodule Hephaestus.Project.V1.RecipeInstallProgress do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.project.v1.RecipeInstallProgress",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RECIPE_INSTALL_PROGRESS_UNSPECIFIED, 0)
  field(:RECIPE_INSTALL_PROGRESS_PENDING, 1)
  field(:RECIPE_INSTALL_PROGRESS_CREATING, 2)
  field(:RECIPE_INSTALL_PROGRESS_READY, 3)
  field(:RECIPE_INSTALL_PROGRESS_FAILED, 4)
  field(:RECIPE_INSTALL_PROGRESS_RECOVERY_REQUIRED, 5)
end

defmodule Hephaestus.Project.V1.RecipeRemovalProgress do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.project.v1.RecipeRemovalProgress",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RECIPE_REMOVAL_PROGRESS_UNSPECIFIED, 0)
  field(:RECIPE_REMOVAL_PROGRESS_PENDING, 1)
  field(:RECIPE_REMOVAL_PROGRESS_DRAINING, 2)
  field(:RECIPE_REMOVAL_PROGRESS_DETACHED, 3)
  field(:RECIPE_REMOVAL_PROGRESS_RETAINED, 4)
  field(:RECIPE_REMOVAL_PROGRESS_DELETED, 5)
  field(:RECIPE_REMOVAL_PROGRESS_RECOVERY_REQUIRED, 6)
end

defmodule Hephaestus.Project.V1.RecipeProgressDiagnostic do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.project.v1.RecipeProgressDiagnostic",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RECIPE_PROGRESS_DIAGNOSTIC_UNSPECIFIED, 0)
  field(:RECIPE_PROGRESS_DIAGNOSTIC_PROVIDER_FAILURE, 1)
  field(:RECIPE_PROGRESS_DIAGNOSTIC_PROVIDER_OUTCOME_UNKNOWN, 2)
  field(:RECIPE_PROGRESS_DIAGNOSTIC_DEPENDENCY_UNAVAILABLE, 3)
  field(:RECIPE_PROGRESS_DIAGNOSTIC_AUTHORIZATION_CHANGED, 4)
  field(:RECIPE_PROGRESS_DIAGNOSTIC_DRAIN_PENDING, 5)
  field(:RECIPE_PROGRESS_DIAGNOSTIC_FENCING_UNPROVEN, 6)
  field(:RECIPE_PROGRESS_DIAGNOSTIC_INTENT_CONFLICT, 7)
end

defmodule Hephaestus.Project.V1.RecipeActivationObservation do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.project.v1.RecipeActivationObservation",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:RECIPE_ACTIVATION_OBSERVATION_UNSPECIFIED, 0)
  field(:RECIPE_ACTIVATION_OBSERVATION_UNOBSERVED, 1)
end

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

defmodule Hephaestus.Project.V1.ProjectRepositoryImage do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ProjectRepositoryImage",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:repository_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:key, 3, type: :string)
  field(:display_name, 4, type: :string, json_name: "displayName")
  field(:source_revision, 5, type: :string, json_name: "sourceRevision")
  field(:base_image_reference, 6, type: :string, json_name: "baseImageReference")
  field(:status, 7, type: :string)
  field(:image_reference, 8, type: :string, json_name: "imageReference")
  field(:failure_reason, 9, type: :string, json_name: "failureReason")
  field(:updated_at, 10, type: Google.Protobuf.Timestamp, json_name: "updatedAt")
end

defmodule Hephaestus.Project.V1.ProjectRepositoryImagePreparationEvent do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ProjectRepositoryImagePreparationEvent",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:phase, 1, type: :string)
  field(:outcome, 2, type: :string)
  field(:output_digest, 3, type: :string, json_name: "outputDigest")
  field(:safe_reason, 4, type: :string, json_name: "safeReason")
  field(:occurred_at, 5, type: Google.Protobuf.Timestamp, json_name: "occurredAt")
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

  field(:mailbox_id, 18,
    proto3_optional: true,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "mailboxId"
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

defmodule Hephaestus.Project.V1.ListProjectRepositoryImagesRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ListProjectRepositoryImagesRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Project.V1.ListProjectRepositoryImagesResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.ListProjectRepositoryImagesResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:images, 1, repeated: true, type: Hephaestus.Project.V1.ProjectRepositoryImage)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Project.V1.GetProjectRepositoryImageRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.GetProjectRepositoryImageRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:image_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "imageId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Project.V1.GetProjectRepositoryImageResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.GetProjectRepositoryImageResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:image, 1, type: Hephaestus.Project.V1.ProjectRepositoryImage)

  field(:history, 2,
    repeated: true,
    type: Hephaestus.Project.V1.ProjectRepositoryImagePreparationEvent
  )

  field(:page, 3, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Project.V1.RetryProjectRepositoryImageRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.RetryProjectRepositoryImageRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:context, 1, type: Hephaestus.Common.V1.RequestContext)
  field(:image_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "imageId")
end

defmodule Hephaestus.Project.V1.RetryProjectRepositoryImageResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.RetryProjectRepositoryImageResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:image, 1, type: Hephaestus.Project.V1.ProjectRepositoryImage)
  field(:receipt, 2, type: Hephaestus.Common.V1.MutationReceipt)
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

defmodule Hephaestus.Project.V1.GetRecipeDeploymentRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.GetRecipeDeploymentRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:deployment_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "deploymentId")
end

defmodule Hephaestus.Project.V1.GetRecipeDeploymentResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.GetRecipeDeploymentResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:deployment, 1, type: Hephaestus.Project.V1.RecipeDeployment)
end

defmodule Hephaestus.Project.V1.RecipeDeployment do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.RecipeDeployment",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:project_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:key, 3, type: :string)
  field(:version, 4, type: :uint64)
  field(:lifecycle, 5, type: Hephaestus.Project.V1.RecipeDeploymentLifecycle, enum: true)
  field(:resources, 6, repeated: true, type: Hephaestus.Project.V1.RecipeDeploymentResource)
  field(:activation, 7, type: Hephaestus.Project.V1.RecipeActivationObservation, enum: true)
end

defmodule Hephaestus.Project.V1.RecipeDeploymentResource do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.project.v1.RecipeDeploymentResource",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:name, 1, type: :string)
  field(:kind, 2, type: Hephaestus.Project.V1.RecipeResourceKind, enum: true)
  field(:resource_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "resourceId")
  field(:revision_id, 4, type: Hephaestus.Common.V1.OpaqueId, json_name: "revisionId")
  field(:ownership, 5, type: Hephaestus.Project.V1.RecipeResourceOwnership, enum: true)
  field(:install, 6, type: Hephaestus.Project.V1.RecipeInstallProgress, enum: true)
  field(:removal, 7, type: Hephaestus.Project.V1.RecipeRemovalProgress, enum: true)
  field(:version, 8, type: :uint64)
  field(:active_attempt_id, 9, type: Hephaestus.Common.V1.OpaqueId, json_name: "activeAttemptId")
  field(:diagnostic, 10, type: Hephaestus.Project.V1.RecipeProgressDiagnostic, enum: true)
  field(:recovery_required, 11, type: :bool, json_name: "recoveryRequired")
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
    :GetRecipeDeployment,
    Hephaestus.Project.V1.GetRecipeDeploymentRequest,
    Hephaestus.Project.V1.GetRecipeDeploymentResponse
  )

  rpc(
    :ListProjectRepositories,
    Hephaestus.Project.V1.ListProjectRepositoriesRequest,
    Hephaestus.Project.V1.ListProjectRepositoriesResponse
  )

  rpc(
    :ListProjectRepositoryImages,
    Hephaestus.Project.V1.ListProjectRepositoryImagesRequest,
    Hephaestus.Project.V1.ListProjectRepositoryImagesResponse
  )

  rpc(
    :GetProjectRepositoryImage,
    Hephaestus.Project.V1.GetProjectRepositoryImageRequest,
    Hephaestus.Project.V1.GetProjectRepositoryImageResponse
  )

  rpc(
    :RetryProjectRepositoryImage,
    Hephaestus.Project.V1.RetryProjectRepositoryImageRequest,
    Hephaestus.Project.V1.RetryProjectRepositoryImageResponse
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
