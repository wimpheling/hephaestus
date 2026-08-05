defmodule HephaestusWebWeb.ProjectBuildersStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.ProjectBuildersState

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
    assert @covered_statuses == ProjectBuildersState.statuses()
    assert ProjectBuildersState.stream_mode() == :none
  end

  test "loads project-scoped builders and ignores stale generations" do
    state = ProjectBuildersState.new(%{project_id: "project-1"})
    {loading, [:load]} = ProjectBuildersState.reduce(state, :load)

    {ready, []} =
      ProjectBuildersState.reduce(
        loading,
        {:loaded, loading.stream_generation, [%{"key" => "typescript-node-ubuntu"}]}
      )

    assert ready.status == :ready
    assert ready.data.builders == [%{"key" => "typescript-node-ubuntu"}]
    assert ProjectBuildersState.reduce(ready, {:loaded, 99, []}) == {ready, []}

    assert Map.keys(Map.from_struct(ProjectBuildersState.new(%{project_id: "project-1"})))
           |> Enum.sort() ==
             [:cursor, :data, :error, :form, :status, :stream_generation]
  end

  test "presents failure states without losing the project scope" do
    state = ProjectBuildersState.new(%{project_id: "project-1"})
    assert ProjectBuildersState.present(state).project_id == "project-1"

    {failed, []} = ProjectBuildersState.reduce(state, {:failed, :unavailable})
    assert ProjectBuildersState.present(failed).state == :error
  end
end
