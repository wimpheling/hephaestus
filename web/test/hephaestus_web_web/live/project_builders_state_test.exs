defmodule HephaestusWebWeb.ProjectBuildersStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.ProjectBuildersState

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
  end

  test "presents reconnect and failure states without losing the project scope" do
    state = ProjectBuildersState.new(%{project_id: "project-1"})
    assert ProjectBuildersState.present(state).project_id == "project-1"

    reconnecting = ProjectBuildersState.reduce(state, :reconnecting)
    assert ProjectBuildersState.present(reconnecting).state == :reconnecting

    {failed, []} = ProjectBuildersState.reduce(state, {:failed, :unavailable})
    assert ProjectBuildersState.present(failed).state == :error
  end
end
