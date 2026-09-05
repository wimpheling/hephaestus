defmodule HephaestusWebWeb.ProjectRepositoryImageStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.ProjectRepositoryImageState

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

  test "uses the standard reauthorizing detail-state contract" do
    assert @covered_statuses == ProjectRepositoryImageState.statuses()
    assert :page_scoped == ProjectRepositoryImageState.stream_mode()

    state = ProjectRepositoryImageState.new(%{project_id: "project-1", image_id: "image-1"})
    assert %{status: :loading} = ProjectRepositoryImageState.present(state)
  end
end
