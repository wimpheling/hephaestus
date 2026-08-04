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
    assert ProjectBuildersState.stream_mode() == :page_scoped
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

  test "presents reconnect and failure states without losing the project scope" do
    state = ProjectBuildersState.new(%{project_id: "project-1"})
    assert ProjectBuildersState.present(state).project_id == "project-1"

    {reconnecting, []} = ProjectBuildersState.reduce(state, :reconnecting)
    assert ProjectBuildersState.present(reconnecting).state == :reconnecting

    {failed, []} = ProjectBuildersState.reduce(state, {:failed, :unavailable})
    assert ProjectBuildersState.present(failed).state == :error
  end

  test "refreshes a project snapshot after committed publication changes and handles reconnects" do
    state =
      %{project_id: "project-1"}
      |> ProjectBuildersState.new()
      |> ProjectBuildersState.begin_watch()

    {awaiting_snapshot, [:snapshot]} =
      ProjectBuildersState.reduce(
        state,
        {:watch,
         %{
           item: {:snapshot_barrier, %{cursor: "cursor-1", versions: %{}}}
         }}
      )

    {ready, []} =
      ProjectBuildersState.reduce(
        awaiting_snapshot,
        {:loaded, awaiting_snapshot.stream_generation, [%{"key" => "typescript-node-ubuntu"}]}
      )

    {stale, [:snapshot]} =
      ProjectBuildersState.reduce(
        ready,
        {:watch,
         %{
           cursor: "cursor-2",
           item:
             {:event,
              %{
                id: "publication-event-1",
                cursor: "cursor-2",
                aggregate_type: "registry_publication",
                aggregate_id: "publication-1",
                aggregate_version: 1,
                payload: {:registry_publication_changed, %{}}
              }}
         }}
      )

    assert stale.status == :stale
    assert stale.data.builders == [%{"key" => "typescript-node-ubuntu"}]

    {reconnecting, [:replace_watch]} = ProjectBuildersState.reduce(stale, :watch_ended)
    assert ProjectBuildersState.present(reconnecting).state == :reconnecting

    {denied, [{:navigate, :organizations}]} =
      ProjectBuildersState.reduce(
        ready,
        {:watch, %{item: {:access_revoked, %{}}}}
      )

    assert denied.status == :access_revoked
  end
end
