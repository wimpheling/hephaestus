defmodule Hephaestus.Image.V1.ImagePreparationState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.image.v1.ImagePreparationState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:IMAGE_PREPARATION_STATE_UNSPECIFIED, 0)
  field(:IMAGE_PREPARATION_STATE_READY, 1)
  field(:IMAGE_PREPARATION_STATE_PREPARING, 2)
  field(:IMAGE_PREPARATION_STATE_FAILED, 3)
end

defmodule Hephaestus.Image.V1.ImageAvailabilityState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.image.v1.ImageAvailabilityState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:IMAGE_AVAILABILITY_STATE_UNSPECIFIED, 0)
  field(:IMAGE_AVAILABILITY_STATE_AVAILABLE, 1)
  field(:IMAGE_AVAILABILITY_STATE_UNAVAILABLE, 2)
  field(:IMAGE_AVAILABILITY_STATE_RETIRED, 3)
end

defmodule Hephaestus.Image.V1.RegistryPublicationState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.image.v1.RegistryPublicationState",
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

defmodule Hephaestus.Image.V1.RegistryAvailabilityState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.image.v1.RegistryAvailabilityState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:REGISTRY_AVAILABILITY_STATE_UNSPECIFIED, 0)
  field(:REGISTRY_AVAILABILITY_STATE_AVAILABLE, 1)
  field(:REGISTRY_AVAILABILITY_STATE_UNAVAILABLE, 2)
  field(:REGISTRY_AVAILABILITY_STATE_RETIRED, 3)
end

defmodule Hephaestus.Image.V1.RegistryEvidenceState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.image.v1.RegistryEvidenceState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:REGISTRY_EVIDENCE_STATE_UNSPECIFIED, 0)
  field(:REGISTRY_EVIDENCE_STATE_PENDING, 1)
  field(:REGISTRY_EVIDENCE_STATE_VERIFIED, 2)
  field(:REGISTRY_EVIDENCE_STATE_NOT_REQUIRED, 3)
end

defmodule Hephaestus.Image.V1.Toolchain do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.image.v1.Toolchain",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:name, 1, type: :string)
  field(:version, 2, type: :string)
end

defmodule Hephaestus.Image.V1.Provenance do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.image.v1.Provenance",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:source, 1, type: :string)
  field(:signature, 2, proto3_optional: true, type: :string)
  field(:sbom, 3, proto3_optional: true, type: :string)
end

defmodule Hephaestus.Image.V1.RegistryEvidence do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.image.v1.RegistryEvidence",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:state, 1, type: Hephaestus.Image.V1.RegistryEvidenceState, enum: true)

  field(:immutable_reference, 2,
    proto3_optional: true,
    type: :string,
    json_name: "immutableReference"
  )
end

defmodule Hephaestus.Image.V1.RegistryPublication do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.image.v1.RegistryPublication",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:state, 1, type: Hephaestus.Image.V1.RegistryPublicationState, enum: true)
  field(:availability, 2, type: Hephaestus.Image.V1.RegistryAvailabilityState, enum: true)

  field(:immutable_reference, 3,
    proto3_optional: true,
    type: :string,
    json_name: "immutableReference"
  )

  field(:architectures, 4, repeated: true, type: :string)
  field(:sbom, 5, type: Hephaestus.Image.V1.RegistryEvidence)
  field(:provenance, 6, type: Hephaestus.Image.V1.RegistryEvidence)
  field(:scan, 7, type: Hephaestus.Image.V1.RegistryEvidence)
  field(:signature, 8, type: Hephaestus.Image.V1.RegistryEvidence)
end

defmodule Hephaestus.Image.V1.OciImage do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.image.v1.OciImage",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:key, 2, type: :string)
  field(:display_name, 3, type: :string, json_name: "displayName")
  field(:image_reference, 4, type: :string, json_name: "imageReference")
  field(:toolchains, 5, repeated: true, type: Hephaestus.Image.V1.Toolchain)
  field(:architectures, 6, repeated: true, type: :string)
  field(:preparation, 7, type: Hephaestus.Image.V1.ImagePreparationState, enum: true)
  field(:availability, 8, type: Hephaestus.Image.V1.ImageAvailabilityState, enum: true)
  field(:provenance, 9, type: Hephaestus.Image.V1.Provenance)
  field(:platform_policy_version, 10, type: :string, json_name: "platformPolicyVersion")

  field(:registry_publication, 11,
    type: Hephaestus.Image.V1.RegistryPublication,
    json_name: "registryPublication"
  )
end

defmodule Hephaestus.Image.V1.ListImagesRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.image.v1.ListImagesRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:page, 1, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.Image.V1.ListImagesResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.image.v1.ListImagesResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:images, 1, repeated: true, type: Hephaestus.Image.V1.OciImage)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.Image.V1.GetImageRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.image.v1.GetImageRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:image_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "imageId")
end

defmodule Hephaestus.Image.V1.GetImageResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.image.v1.GetImageResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:image, 1, type: Hephaestus.Image.V1.OciImage)
end

defmodule Hephaestus.Image.V1.ImageCatalogService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.image.v1.ImageCatalogService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(:ListImages, Hephaestus.Image.V1.ListImagesRequest, Hephaestus.Image.V1.ListImagesResponse)

  rpc(:GetImage, Hephaestus.Image.V1.GetImageRequest, Hephaestus.Image.V1.GetImageResponse)
end

defmodule Hephaestus.Image.V1.ImageCatalogService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Image.V1.ImageCatalogService.Service
end
