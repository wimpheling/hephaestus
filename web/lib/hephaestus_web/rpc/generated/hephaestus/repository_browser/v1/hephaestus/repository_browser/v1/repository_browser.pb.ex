defmodule Hephaestus.RepositoryBrowser.V1.TreeEntryType do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.repository_browser.v1.TreeEntryType",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:TREE_ENTRY_TYPE_UNSPECIFIED, 0)
  field(:TREE_ENTRY_TYPE_BLOB, 1)
  field(:TREE_ENTRY_TYPE_TREE, 2)
  field(:TREE_ENTRY_TYPE_COMMIT, 3)
end

defmodule Hephaestus.RepositoryBrowser.V1.Branch do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.Branch",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:name, 1, type: :string)
  field(:ref, 2, type: :string)
  field(:commit, 3, type: :string)
  field(:committed_at, 4, type: Google.Protobuf.Timestamp, json_name: "committedAt")
  field(:subject, 5, type: :string)
end

defmodule Hephaestus.RepositoryBrowser.V1.Commit do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.Commit",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:id, 1, type: :string)
  field(:parents, 2, repeated: true, type: :string)
  field(:author_name, 3, type: :string, json_name: "authorName")
  field(:author_email, 4, type: :string, json_name: "authorEmail")
  field(:authored_at, 5, type: Google.Protobuf.Timestamp, json_name: "authoredAt")
  field(:subject, 6, type: :string)
end

defmodule Hephaestus.RepositoryBrowser.V1.TreeEntry do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.TreeEntry",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:mode, 1, type: :string)
  field(:type, 2, type: Hephaestus.RepositoryBrowser.V1.TreeEntryType, enum: true)
  field(:object_id, 3, type: :string, json_name: "objectId")
  field(:size, 4, proto3_optional: true, type: :uint64)
  field(:path, 5, type: :string)
end

defmodule Hephaestus.RepositoryBrowser.V1.ListBranchesRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.ListBranchesRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:page, 2, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.RepositoryBrowser.V1.ListBranchesResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.ListBranchesResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:branches, 1, repeated: true, type: Hephaestus.RepositoryBrowser.V1.Branch)
  field(:page, 2, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.RepositoryBrowser.V1.ListCommitsRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.ListCommitsRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:branch, 2, type: :string)
  field(:page, 3, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.RepositoryBrowser.V1.ListCommitsResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.ListCommitsResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:selected_branch, 1,
    type: Hephaestus.RepositoryBrowser.V1.Branch,
    json_name: "selectedBranch"
  )

  field(:commits, 2, repeated: true, type: Hephaestus.RepositoryBrowser.V1.Commit)
  field(:page, 3, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.RepositoryBrowser.V1.GetTreeRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.GetTreeRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:branch, 2, type: :string)
  field(:page, 3, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.RepositoryBrowser.V1.GetTreeResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.GetTreeResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:selected_branch, 1,
    type: Hephaestus.RepositoryBrowser.V1.Branch,
    json_name: "selectedBranch"
  )

  field(:entries, 2, repeated: true, type: Hephaestus.RepositoryBrowser.V1.TreeEntry)
  field(:page, 3, type: Hephaestus.Common.V1.PageResponse)
end

defmodule Hephaestus.RepositoryBrowser.V1.GetFileRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.GetFileRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:branch, 2, type: :string)
  field(:path, 3, type: :string)
end

defmodule Hephaestus.RepositoryBrowser.V1.GetFileResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.GetFileResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:entry, 1, type: Hephaestus.RepositoryBrowser.V1.TreeEntry)
  field(:utf8_contents, 2, type: :string, json_name: "utf8Contents")
  field(:language, 3, type: :string)
end

defmodule Hephaestus.RepositoryBrowser.V1.StreamFileRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.StreamFileRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:branch, 2, type: :string)
  field(:path, 3, type: :string)
  field(:resume_cursor, 4, type: Hephaestus.Common.V1.Cursor, json_name: "resumeCursor")
  field(:max_total_bytes, 5, type: :uint64, json_name: "maxTotalBytes")
  field(:max_chunk_bytes, 6, type: :uint32, json_name: "maxChunkBytes")
end

defmodule Hephaestus.RepositoryBrowser.V1.StreamFileResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.StreamFileResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:sequence, 1, type: :uint64)
  field(:contents, 2, type: :bytes)
  field(:committed_cursor, 3, type: Hephaestus.Common.V1.Cursor, json_name: "committedCursor")
  field(:end_of_file, 4, type: :bool, json_name: "endOfFile")
  field(:media_type, 5, type: :string, json_name: "mediaType")
end

defmodule Hephaestus.RepositoryBrowser.V1.RepositoryBrowserService.Service do
  @moduledoc false

  use GRPC.Service,
    name: "hephaestus.repository_browser.v1.RepositoryBrowserService",
    protoc_gen_elixir_version: "0.17.0"

  rpc(
    :ListBranches,
    Hephaestus.RepositoryBrowser.V1.ListBranchesRequest,
    Hephaestus.RepositoryBrowser.V1.ListBranchesResponse
  )

  rpc(
    :ListCommits,
    Hephaestus.RepositoryBrowser.V1.ListCommitsRequest,
    Hephaestus.RepositoryBrowser.V1.ListCommitsResponse
  )

  rpc(
    :GetTree,
    Hephaestus.RepositoryBrowser.V1.GetTreeRequest,
    Hephaestus.RepositoryBrowser.V1.GetTreeResponse
  )

  rpc(
    :GetFile,
    Hephaestus.RepositoryBrowser.V1.GetFileRequest,
    Hephaestus.RepositoryBrowser.V1.GetFileResponse
  )

  rpc(
    :StreamFile,
    Hephaestus.RepositoryBrowser.V1.StreamFileRequest,
    stream(Hephaestus.RepositoryBrowser.V1.StreamFileResponse)
  )
end

defmodule Hephaestus.RepositoryBrowser.V1.RepositoryBrowserService.Stub do
  @moduledoc false

  use GRPC.Stub, service: Hephaestus.RepositoryBrowser.V1.RepositoryBrowserService.Service
end
