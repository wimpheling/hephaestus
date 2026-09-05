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

defmodule Hephaestus.RepositoryBrowser.V1.DiffFileState do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.repository_browser.v1.DiffFileState",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:DIFF_FILE_STATE_UNSPECIFIED, 0)
  field(:DIFF_FILE_STATE_ADDED, 1)
  field(:DIFF_FILE_STATE_DELETED, 2)
  field(:DIFF_FILE_STATE_MODIFIED, 3)
  field(:DIFF_FILE_STATE_RENAMED, 4)
  field(:DIFF_FILE_STATE_BINARY, 5)
  field(:DIFF_FILE_STATE_TRUNCATED, 6)
  field(:DIFF_FILE_STATE_UNAVAILABLE, 7)
end

defmodule Hephaestus.RepositoryBrowser.V1.DiffLineKind do
  @moduledoc false

  use Protobuf,
    enum: true,
    full_name: "hephaestus.repository_browser.v1.DiffLineKind",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:DIFF_LINE_KIND_UNSPECIFIED, 0)
  field(:DIFF_LINE_KIND_CONTEXT, 1)
  field(:DIFF_LINE_KIND_ADDED, 2)
  field(:DIFF_LINE_KIND_REMOVED, 3)
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

defmodule Hephaestus.RepositoryBrowser.V1.DiffLine do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.DiffLine",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:kind, 1, type: Hephaestus.RepositoryBrowser.V1.DiffLineKind, enum: true)
  field(:old_line, 2, proto3_optional: true, type: :uint32, json_name: "oldLine")
  field(:new_line, 3, proto3_optional: true, type: :uint32, json_name: "newLine")
  field(:text, 4, type: :string)
end

defmodule Hephaestus.RepositoryBrowser.V1.DiffHunk do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.DiffHunk",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:old_start, 1, type: :uint32, json_name: "oldStart")
  field(:old_lines, 2, type: :uint32, json_name: "oldLines")
  field(:new_start, 3, type: :uint32, json_name: "newStart")
  field(:new_lines, 4, type: :uint32, json_name: "newLines")
  field(:lines, 5, repeated: true, type: Hephaestus.RepositoryBrowser.V1.DiffLine)
  field(:truncated, 6, type: :bool)
end

defmodule Hephaestus.RepositoryBrowser.V1.DiffFile do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.DiffFile",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:path, 1, type: :string)
  field(:previous_path, 2, type: :string, json_name: "previousPath")
  field(:state, 3, type: Hephaestus.RepositoryBrowser.V1.DiffFileState, enum: true)
  field(:additions, 4, type: :uint64)
  field(:deletions, 5, type: :uint64)
  field(:hunks, 6, repeated: true, type: Hephaestus.RepositoryBrowser.V1.DiffHunk)
  field(:binary, 7, type: :bool)
  field(:truncated, 8, type: :bool)
end

defmodule Hephaestus.RepositoryBrowser.V1.CommitDetail do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.CommitDetail",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:commit, 1, type: Hephaestus.RepositoryBrowser.V1.Commit)
  field(:committer_name, 2, type: :string, json_name: "committerName")
  field(:committer_email, 3, type: :string, json_name: "committerEmail")
  field(:committed_at, 4, type: Google.Protobuf.Timestamp, json_name: "committedAt")
  field(:body, 5, type: :string)
  field(:selected_parent, 6, type: :string, json_name: "selectedParent")
  field(:files, 7, repeated: true, type: Hephaestus.RepositoryBrowser.V1.DiffFile)
  field(:truncated, 8, type: :bool)
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

defmodule Hephaestus.RepositoryBrowser.V1.GetCommitDetailRequest do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.GetCommitDetailRequest",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:repository_id, 1, type: Hephaestus.Common.V1.OpaqueId, json_name: "repositoryId")
  field(:branch, 2, type: :string)
  field(:commit, 3, type: :string)
  field(:parent, 4, type: :string)
  field(:page, 5, type: Hephaestus.Common.V1.PageRequest)
end

defmodule Hephaestus.RepositoryBrowser.V1.GetCommitDetailResponse do
  @moduledoc false

  use Protobuf,
    full_name: "hephaestus.repository_browser.v1.GetCommitDetailResponse",
    protoc_gen_elixir_version: "0.17.0",
    syntax: :proto3

  field(:selected_branch, 1,
    type: Hephaestus.RepositoryBrowser.V1.Branch,
    json_name: "selectedBranch"
  )

  field(:detail, 2, type: Hephaestus.RepositoryBrowser.V1.CommitDetail)
  field(:page, 3, type: Hephaestus.Common.V1.PageResponse)
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
    :GetCommitDetail,
    Hephaestus.RepositoryBrowser.V1.GetCommitDetailRequest,
    Hephaestus.RepositoryBrowser.V1.GetCommitDetailResponse
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
