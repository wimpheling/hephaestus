defmodule HephaestusWebWeb.ProjectRepositoryImagesStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.ProjectRepositoryImagesState

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

  test "uses the standard state contract and renders a safe failure" do
    assert @covered_statuses == ProjectRepositoryImagesState.statuses()
    assert :none == ProjectRepositoryImagesState.stream_mode()

    state = ProjectRepositoryImagesState.new(%{project_id: "project-1"})
    assert %{status: :loading} = ProjectRepositoryImagesState.present(state)

    {state, _effects} =
      ProjectRepositoryImagesState.reduce(state, {:error, HephaestusWeb.RPC.Error.unavailable()})

    assert %{status: :error, error: "Image operation could not be completed."} =
             ProjectRepositoryImagesState.present(state)
  end
end
