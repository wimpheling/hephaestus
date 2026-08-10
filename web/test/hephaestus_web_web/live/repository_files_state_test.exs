defmodule HephaestusWebWeb.RepositoryFilesStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWebWeb.RepositoryFilesState

  @covered_statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]

  test "covers lifecycle, reconnect, navigation, and stale generations" do
    state = RepositoryFilesState.new("repository-1")
    assert RepositoryFilesState.statuses() == @covered_statuses

    {loading, [_effect]} =
      RepositoryFilesState.reduce(state, {:load, %{}, "/repositories/repository-1/files"})

    assert RepositoryFilesState.reduce(loading, {:loaded, 0, {:error, :stale}}) == {loading, []}

    assert {_, [{:patch, "/repositories/repository-1/files?ref=main"}]} =
             RepositoryFilesState.reduce(loading, {:select_branch, "main"})

    assert {reconnecting, []} = RepositoryFilesState.reduce(loading, :disconnected)
    assert RepositoryFilesState.present(reconnecting).state == :reconnecting
  end

  test "presents the canonical remote and default branch for an empty repository" do
    state = RepositoryFilesState.new("repository-1")

    {loading, _effects} =
      RepositoryFilesState.reduce(
        state,
        {:load, %{}, "https://forge.example/repositories/repository-1/files"}
      )

    {ready, []} =
      RepositoryFilesState.reduce(
        loading,
        {:loaded, loading.stream_generation,
         {:ok,
          %{
            repository_id: "repository-1",
            repository: %{
              "id" => "repository-1",
              "organization_id" => "organization-1",
              "project_id" => "project-1",
              "organization_name" => "Acme",
              "project_name" => "Forge",
              "name" => "Source",
              "default_branch" => "refs/heads/trunk",
              "is_public" => false
            },
            remote_url: "https://forge.example/repository-1",
            selected_branch: nil,
            branch_options: [],
            branches: [],
            commits: [],
            builds: [],
            releases: [],
            attached_instances: [],
            tree: %{name: "", path: "", directories: [], files: [], file_count: 0},
            current_path: nil,
            file: nil,
            file_error: nil,
            params: %{},
            uri: "/repositories/repository-1/files"
          }}}
      )

    presentation = RepositoryFilesState.present(ready)
    assert presentation.remote_url == "https://forge.example/repository-1"
    assert presentation.default_branch == "trunk"
    assert presentation.branches_empty?
  end
end
