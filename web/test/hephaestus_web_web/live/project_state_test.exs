defmodule HephaestusWebWeb.ProjectStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWebWeb.ProjectState

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

  test "rejects stale repository generations" do
    assert @covered_statuses == ProjectState.statuses()
    state = ProjectState.new(%{project_id: "project-1"})
    {loading, [:load]} = ProjectState.reduce(state, {:load, 2})
    {unchanged, []} = ProjectState.reduce(loading, {:loaded, 1, %{}, [%{"id" => "old"}]})
    assert unchanged == loading
  end
end
