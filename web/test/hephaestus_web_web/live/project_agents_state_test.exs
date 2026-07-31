defmodule HephaestusWebWeb.ProjectAgentsStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWebWeb.ProjectAgentsState

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

  test "keeps sensitive import parameters outside state" do
    assert @covered_statuses == ProjectAgentsState.statuses()
    state = ProjectAgentsState.new(%{project_id: "project-1"})
    refute Map.has_key?(state, :parameters)
    assert ProjectAgentsState.stream_mode() == :page_scoped
  end
end
