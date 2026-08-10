defmodule HephaestusWebWeb.RepositoryBuildsStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.RepositoryBuildsState

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

  test "declares the complete lifecycle" do
    assert @covered_statuses == RepositoryBuildsState.statuses()
  end

  test "presents the Builds tab with the typed list API" do
    state = RepositoryBuildsState.new("repository-1")

    {loading, [_effect]} =
      RepositoryBuildsState.reduce(state, {:load, %{}, "/repositories/repository-1/builds"})

    assert {ready, []} =
             RepositoryBuildsState.reduce(
               loading,
               {:loaded, loading.stream_generation,
                {:ok,
                 %{
                   repository_id: "repository-1",
                   repository: %{"id" => "repository-1", "name" => "Source"},
                   selected_branch: nil,
                   branch_options: [],
                   branches: [],
                   commits: [],
                   releases: [],
                   builds: [],
                   builds_unavailable?: false,
                   attached_instances: [],
                   params: %{},
                   uri: "/repositories/repository-1/builds"
                 }}}
             )

    presentation = RepositoryBuildsState.present(ready)
    assert presentation.state == :ready
    assert presentation.active == :builds
    refute presentation.builds_unavailable?
    assert Enum.any?(presentation.tabs, &(&1.key == :builds))
  end

  test "validates and requests a manually supplied exact build input" do
    state = RepositoryBuildsState.new("repository-1")

    {submitting_invalid, [{:request_build, "repository-1", %{"source_commit" => ""}}]} =
      RepositoryBuildsState.reduce(state, {:request_build, %{"source_commit" => ""}})

    assert submitting_invalid.status == :submitting

    attributes = %{
      "source_commit" => "commit-1",
      "build_definition_hash" => "definition-1",
      "configuration_hash" => "configuration-1"
    }

    {submitting, [{:request_build, "repository-1", ^attributes}]} =
      RepositoryBuildsState.reduce(state, {:request_build, attributes})

    assert submitting.status == :submitting
  end

  test "navigates to a requested build and rejects stale results" do
    state = RepositoryBuildsState.new("repository-1")

    {submitting, _} =
      RepositoryBuildsState.reduce(state, {:request_build, valid_attributes()})

    assert RepositoryBuildsState.reduce(submitting, {:loaded, 0, {:error, :stale}}) ==
             {submitting, []}

    {ready, [{:navigate, destination}]} =
      RepositoryBuildsState.reduce(
        submitting,
        {:request_build_result, {:ok, %{"build_id" => "build-1"}}}
      )

    assert ready.status == :ready
    assert destination == "/repositories/repository-1/builds/build-1"
  end

  defp valid_attributes do
    %{
      "source_commit" => "commit-1",
      "build_definition_hash" => "definition-1",
      "configuration_hash" => "configuration-1"
    }
  end
end
