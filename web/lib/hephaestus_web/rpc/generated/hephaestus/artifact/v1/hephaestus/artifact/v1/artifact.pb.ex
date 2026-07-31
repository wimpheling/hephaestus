defmodule Hephaestus.Artifact.V1.ArtifactProvenance do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.artifact.v1.ArtifactProvenance",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:build_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "buildId")
  field(:release_id, 2, type: Hephaestus.Common.V1.OpaqueId, json_name: "releaseId")
  field(:run_id, 3, type: Hephaestus.Common.V1.OpaqueId, json_name: "runId")
  field(:source_commit, 4, type: :string, json_name: "sourceCommit")
end

defmodule Hephaestus.Artifact.V1.Artifact do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.artifact.v1.Artifact",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: Hephaestus.Common.V1.OpaqueId)
  field(:path, 2, type: :string)
  field(:kind, 3, type: :string)
  field(:mode, 4, type: :uint32)
  field(:sha256, 5, type: :string)
  field(:size_bytes, 6, type: :uint64, json_name: "sizeBytes")
  field(:media_type, 7, type: :string, json_name: "mediaType")
  field(:provenance, 8, type: Hephaestus.Artifact.V1.ArtifactProvenance)
end

defmodule Hephaestus.Artifact.V1.GetArtifactPreviewRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.artifact.v1.GetArtifactPreviewRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:artifact_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "artifactId")
  field(:max_bytes, 2, type: :uint32, json_name: "maxBytes")
end

defmodule Hephaestus.Artifact.V1.GetArtifactPreviewResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.artifact.v1.GetArtifactPreviewResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:artifact, 1, type: Hephaestus.Artifact.V1.Artifact)
  field(:utf8_contents, 2, type: :string, json_name: "utf8Contents")
  field(:truncated, 3, type: :bool)
end

defmodule Hephaestus.Artifact.V1.StreamArtifactRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.artifact.v1.StreamArtifactRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:artifact_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "artifactId")
  field(:resume_cursor, 2, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_total_bytes, 3, type: :uint64, json_name: "maxTotalBytes")
  field(:max_chunk_bytes, 4, type: :uint32, json_name: "maxChunkBytes")
end

defmodule Hephaestus.Artifact.V1.StreamArtifactResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.artifact.v1.StreamArtifactResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:sequence, 1, type: :uint64)
  field(:contents, 2, type: :bytes)
  field(:committed_cursor, 3, type: Hephaestus.Common.V1.Cursor, json_name: "committedCursor")
  field(:end_of_artifact, 4, type: :bool, json_name: "endOfArtifact")
  field(:media_type, 5, type: :string, json_name: "mediaType")
end

defmodule Hephaestus.Artifact.V1.ArtifactService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.artifact.v1.ArtifactService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :GetArtifactPreview,
    Hephaestus.Artifact.V1.GetArtifactPreviewRequest,
    Hephaestus.Artifact.V1.GetArtifactPreviewResponse
  )

  rpc(
    :StreamArtifact,
    Hephaestus.Artifact.V1.StreamArtifactRequest,
    stream(Hephaestus.Artifact.V1.StreamArtifactResponse)
  )
end

defmodule Hephaestus.Artifact.V1.ArtifactService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.Artifact.V1.ArtifactService.Service
end
