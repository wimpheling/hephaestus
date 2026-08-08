defmodule HephaestusWebWeb.RepositoryNewStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.RepositoryNewState

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

  test "declares the complete lifecycle and renders project context" do
    assert @covered_statuses == RepositoryNewState.statuses()
    state = RepositoryNewState.new(%{project_id: "project-1"})
    {loading, [:load]} = RepositoryNewState.reduce(state, :load)

    {ready, []} =
      RepositoryNewState.reduce(loading, {:loaded, {:ok, %{"id" => "project-1"}}})

    assert RepositoryNewState.present(ready).state == :ready
  end

  test "turns service results into navigation or useful errors" do
    state = RepositoryNewState.new(%{project_id: "project-1"})

    {ready, [{:flash, :info, _}, {:navigate, "/repositories/repository-1"}]} =
      RepositoryNewState.reduce(state, {:created, {:ok, %{"repository_id" => "repository-1"}}})

    assert ready.status == :ready

    {failed, [{:flash, :error, message}]} =
      RepositoryNewState.reduce(state, {:failed, :invalid})

    assert failed.status == :error
    assert message != ""
  end
end
