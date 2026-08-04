defmodule Hephaestus.Builder.V1.PreparationState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.builder.v1.PreparationState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:PREPARATION_STATE_UNSPECIFIED, 0)
  field(:PREPARATION_STATE_READY, 1)
  field(:PREPARATION_STATE_PREPARING, 2)
  field(:PREPARATION_STATE_FAILED, 3)
end

defmodule Hephaestus.Builder.V1.AvailabilityState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.builder.v1.AvailabilityState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:AVAILABILITY_STATE_UNSPECIFIED, 0)
  field(:AVAILABILITY_STATE_AVAILABLE, 1)
  field(:AVAILABILITY_STATE_UNAVAILABLE, 2)
  field(:AVAILABILITY_STATE_RETIRED, 3)
end

defmodule Hephaestus.Builder.V1.RegistryPublicationState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.builder.v1.RegistryPublicationState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:REGISTRY_PUBLICATION_STATE_UNSPECIFIED, 0)
  field(:REGISTRY_PUBLICATION_STATE_NOT_REQUESTED, 1)
  field(:REGISTRY_PUBLICATION_STATE_PENDING, 2)
  field(:REGISTRY_PUBLICATION_STATE_PUBLISHING, 3)
  field(:REGISTRY_PUBLICATION_STATE_VERIFIED, 4)
  field(:REGISTRY_PUBLICATION_STATE_APPROVED, 5)
  field(:REGISTRY_PUBLICATION_STATE_MISSING, 6)
  field(:REGISTRY_PUBLICATION_STATE_RETIRED, 7)
  field(:REGISTRY_PUBLICATION_STATE_FAILED, 8)
end

defmodule Hephaestus.Builder.V1.RegistryAvailabilityState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.builder.v1.RegistryAvailabilityState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:REGISTRY_AVAILABILITY_STATE_UNSPECIFIED, 0)
  field(:REGISTRY_AVAILABILITY_STATE_AVAILABLE, 1)
  field(:REGISTRY_AVAILABILITY_STATE_UNAVAILABLE, 2)
  field(:REGISTRY_AVAILABILITY_STATE_RETIRED, 3)
end

defmodule Hephaestus.Builder.V1.RegistryEvidenceState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.builder.v1.RegistryEvidenceState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:REGISTRY_EVIDENCE_STATE_UNSPECIFIED, 0)
  field(:REGISTRY_EVIDENCE_STATE_PENDING, 1)
  field(:REGISTRY_EVIDENCE_STATE_VERIFIED, 2)
  field(:REGISTRY_EVIDENCE_STATE_NOT_REQUIRED, 3)
end

defmodule Hephaestus.Builder.V1.DependencyPolicy do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.builder.v1.DependencyPolicy",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:DEPENDENCY_POLICY_UNSPECIFIED, 0)
  field(:DEPENDENCY_POLICY_VENDORED_OFFLINE, 1)
  field(:DEPENDENCY_POLICY_READ_ONLY_PLATFORM_CACHE, 2)
  field(:DEPENDENCY_POLICY_CONSTRAINED_REGISTRY_EGRESS, 3)
end

defmodule Hephaestus.Builder.V1.ProjectBuilderStatus do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.builder.v1.ProjectBuilderStatus",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:PROJECT_BUILDER_STATUS_UNSPECIFIED, 0)
  field(:PROJECT_BUILDER_STATUS_DRAFT, 1)
  field(:PROJECT_BUILDER_STATUS_PREPARING, 2)
  field(:PROJECT_BUILDER_STATUS_READY, 3)
  field(:PROJECT_BUILDER_STATUS_FAILED, 4)
  field(:PROJECT_BUILDER_STATUS_RETIRED, 5)
end

defmodule Hephaestus.Builder.V1.Toolchain do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.Toolchain",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:name, 1, type: :string)
  field(:version, 2, type: :string)
end

defmodule Hephaestus.Builder.V1.Provenance do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.Provenance",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:source, 1, type: :string)
  field(:signature, 2, proto3_optional: true, type: :string)
  field(:sbom, 3, proto3_optional: true, type: :string)
end

defmodule Hephaestus.Builder.V1.RegistryEvidence do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.RegistryEvidence",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:state, 1, type: Hephaestus.Builder.V1.RegistryEvidenceState, enum: true)

  field(:immutable_reference, 2,
    proto3_optional: true,
    type: :string,
    json_name: "immutableReference"
  )
end

defmodule Hephaestus.Builder.V1.RegistryPublication do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.RegistryPublication",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:state, 1, type: Hephaestus.Builder.V1.RegistryPublicationState, enum: true)
  field(:availability, 2, type: Hephaestus.Builder.V1.RegistryAvailabilityState, enum: true)

  field(:immutable_reference, 3,
    proto3_optional: true,
    type: :string,
    json_name: "immutableReference"
  )

  field(:architectures, 4, repeated: true, type: :string)
  field(:sbom, 5, type: Hephaestus.Builder.V1.RegistryEvidence)
  field(:provenance, 6, type: Hephaestus.Builder.V1.RegistryEvidence)
  field(:scan, 7, type: Hephaestus.Builder.V1.RegistryEvidence)
  field(:signature, 8, type: Hephaestus.Builder.V1.RegistryEvidence)
end

defmodule Hephaestus.Builder.V1.BuilderImage do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.BuilderImage",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:key, 2, type: :string)
  field(:display_name, 3, type: :string, json_name: "displayName")
  field(:image_reference, 4, type: :string, json_name: "imageReference")
  field(:toolchains, 5, repeated: true, type: Hephaestus.Builder.V1.Toolchain)
  field(:architectures, 6, repeated: true, type: :string)
  field(:preparation, 7, type: Hephaestus.Builder.V1.PreparationState, enum: true)
  field(:availability, 8, type: Hephaestus.Builder.V1.AvailabilityState, enum: true)

  field(:network_ceiling, 9,
    type: Hephaestus.Common.V1.NetworkPolicy,
    json_name: "networkCeiling",
    enum: true
  )

  field(:max_vcpus, 10, type: :uint32, json_name: "maxVcpus")
  field(:max_memory_mib, 11, type: :uint32, json_name: "maxMemoryMib")

  field(:dependency_policy, 12,
    type: Hephaestus.Builder.V1.DependencyPolicy,
    json_name: "dependencyPolicy",
    enum: true
  )

  field(:provenance, 13, type: Hephaestus.Builder.V1.Provenance)
  field(:platform_policy_version, 14, type: :string, json_name: "platformPolicyVersion")

  field(:registry_publication, 15,
    type: Hephaestus.Builder.V1.RegistryPublication,
    json_name: "registryPublication"
  )
end

defmodule Hephaestus.Builder.V1.ListBuilderImagesRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.ListBuilderImagesRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:page, 1, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Builder.V1.ListBuilderImagesResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.ListBuilderImagesResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:builder_images, 1,
    repeated: true,
    type: Hephaestus.Builder.V1.BuilderImage,
    json_name: "builderImages"
  )

  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Builder.V1.GetBuilderImageRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.GetBuilderImageRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:builder_image_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "builderImageId")
end

defmodule Hephaestus.Builder.V1.GetBuilderImageResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.GetBuilderImageResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:builder_image, 1, type: Hephaestus.Builder.V1.BuilderImage, json_name: "builderImage")
end

defmodule Hephaestus.Builder.V1.ValidateAgentConfigRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.ValidateAgentConfigRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:agent_toml, 1, type: :bytes, json_name: "agentToml", deprecated: false)
end

defmodule Hephaestus.Builder.V1.ValidateAgentConfigResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.ValidateAgentConfigResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:builder_image, 1, type: Hephaestus.Builder.V1.BuilderImage, json_name: "builderImage")
  field(:network, 2, type: Hephaestus.Common.V1.NetworkPolicy, enum: true)
  field(:vcpus, 3, type: :uint32)
  field(:memory_mib, 4, type: :uint32, json_name: "memoryMib")
  field(:platform_policy_version, 5, type: :string, json_name: "platformPolicyVersion")
end

defmodule Hephaestus.Builder.V1.ProjectBuilderProvenance do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.ProjectBuilderProvenance",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:source_revision, 1, type: :string, json_name: "sourceRevision")
  field(:context_digest, 2, type: :string, json_name: "contextDigest")
  field(:attestation_reference, 3, type: :string, json_name: "attestationReference")
  field(:sbom_reference, 4, proto3_optional: true, type: :string, json_name: "sbomReference")
end

defmodule Hephaestus.Builder.V1.ProjectBuilder do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.ProjectBuilder",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:project_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")

  field(:source_repository_id, 3,
    type: Hephaestus.Common.V1.OpaqueId,
    json_name: "sourceRepositoryId"
  )

  field(:key, 4, type: :string)
  field(:display_name, 5, type: :string, json_name: "displayName")
  field(:source_revision, 6, type: :string, json_name: "sourceRevision")
  field(:dockerfile_path, 7, type: :string, json_name: "dockerfilePath")
  field(:context_path, 8, type: :string, json_name: "contextPath")
  field(:context_digest, 9, type: :string, json_name: "contextDigest")

  field(:approved_base_image_reference, 10,
    type: :string,
    json_name: "approvedBaseImageReference"
  )

  field(:status, 11, type: Hephaestus.Builder.V1.ProjectBuilderStatus, enum: true)

  field(:oci_image_reference, 12,
    proto3_optional: true,
    type: :string,
    json_name: "ociImageReference"
  )

  field(:oci_image_digest, 13, proto3_optional: true, type: :string, json_name: "ociImageDigest")

  field(:provenance, 14,
    proto3_optional: true,
    type: Hephaestus.Builder.V1.ProjectBuilderProvenance
  )

  field(:failure_reason, 15, proto3_optional: true, type: :string, json_name: "failureReason")
  field(:created_at, 16, type: Google.Protobuf.Timestamp, json_name: "createdAt")
  field(:updated_at, 17, type: Google.Protobuf.Timestamp, json_name: "updatedAt")

  field(:registry_publication, 18,
    type: Hephaestus.Builder.V1.RegistryPublication,
    json_name: "registryPublication"
  )
end

defmodule Hephaestus.Builder.V1.ListProjectBuildersRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.ListProjectBuildersRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Builder.V1.ListProjectBuildersResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.ListProjectBuildersResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:builders, 1, repeated: true, type: Hephaestus.Builder.V1.ProjectBuilder)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Builder.V1.GetProjectBuilderRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.GetProjectBuilderRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:project_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "projectId")
  field(:builder_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "builderId")
end

defmodule Hephaestus.Builder.V1.GetProjectBuilderResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.builder.v1.GetProjectBuilderResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:builder, 1, type: Hephaestus.Builder.V1.ProjectBuilder)
end

defmodule Hephaestus.Builder.V1.BuilderCatalogService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.builder.v1.BuilderCatalogService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :ListBuilderImages,
    Hephaestus.Builder.V1.ListBuilderImagesRequest,
    Hephaestus.Builder.V1.ListBuilderImagesResponse
  )

  rpc(
    :GetBuilderImage,
    Hephaestus.Builder.V1.GetBuilderImageRequest,
    Hephaestus.Builder.V1.GetBuilderImageResponse
  )

  rpc(
    :ValidateAgentConfig,
    Hephaestus.Builder.V1.ValidateAgentConfigRequest,
    Hephaestus.Builder.V1.ValidateAgentConfigResponse
  )

  rpc(
    :ListProjectBuilders,
    Hephaestus.Builder.V1.ListProjectBuildersRequest,
    Hephaestus.Builder.V1.ListProjectBuildersResponse
  )

  rpc(
    :GetProjectBuilder,
    Hephaestus.Builder.V1.GetProjectBuilderRequest,
    Hephaestus.Builder.V1.GetProjectBuilderResponse
  )
end

defmodule Hephaestus.Builder.V1.BuilderCatalogService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Builder.V1.BuilderCatalogService.Service
end
