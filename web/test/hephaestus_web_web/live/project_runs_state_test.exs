defmodule HephaestusWebWeb.ProjectRunsStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWebWeb.ProjectRunsState

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

  test "constructs a presentation for every status" do
    assert @covered_statuses == ProjectRunsState.statuses()

    for status <- @covered_statuses do
      state = %{ProjectRunsState.new(%{project_id: "project-1"}) | status: status}

      assert ProjectRunsState.present(state).status in [
               :loading,
               :empty,
               :error,
               :reconnecting,
               :ready
             ]
    end
  end
end
