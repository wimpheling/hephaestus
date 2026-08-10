defmodule HephaestusWebWeb.ProjectGatewayStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.{ProductEventReducer, ProjectGatewayState, ProjectGatewaysState}

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

  test "covers the gateway detail lifecycle contract" do
    assert @covered_statuses == ProjectGatewayState.statuses()
    assert ProjectGatewayState.stream_mode() == :page_scoped
  end

  test "gateway list treats only gateway invalidations as a snapshot trigger" do
    state = barrier_ready(ProjectGatewaysState.new(%{project_id: "project-1"}))

    {_ignored, []} =
      ProjectGatewaysState.reduce(state, {:watch, event(:repository_changed, "repository")})

    {_changed, [:snapshot]} =
      ProjectGatewaysState.reduce(state, {:watch, event(:gateway_changed, "gateway")})
  end

  test "gateway detail waits for its lifecycle receipt before refreshing" do
    state =
      barrier_ready(ProjectGatewayState.new(%{project_id: "project-1", gateway_id: "gateway-1"}))

    {waiting, []} =
      ProjectGatewayState.reduce(state, {
        :transitioned,
        %{
          "gateway" => %{"id" => "gateway-1", "lifecycle" => "paused"},
          "receipt" => %{
            "committed_cursor" => "cursor-2",
            "event_id" => "gateway-event",
            "aggregate_version" => 2
          }
        }
      })

    assert waiting.status == :stale

    {_changed, [:snapshot]} =
      ProjectGatewayState.reduce(
        waiting,
        {:watch, %{event(:gateway_changed, "gateway") | cursor: "cursor-2"}}
      )
  end

  test "gateway lifecycle rejection reaches an error presentation state" do
    state = ProjectGatewayState.new(%{project_id: "project-1", gateway_id: "gateway-1"})

    {submitting, [{:lifecycle, "paused"}]} =
      ProjectGatewayState.reduce(state, {:lifecycle, "paused"})

    {failed, []} = ProjectGatewayState.reduce(submitting, :lifecycle_failed)

    assert failed.status == :error
    assert ProjectGatewayState.present(failed).status == :error
  end

  defp barrier_ready(state) do
    barrier = %{cursor: "cursor-1", versions: %{}, schema_version: 1}

    {loading, [:snapshot]} =
      ProductEventReducer.reduce(
        state,
        %{cursor: "cursor-1", item: {:snapshot_barrier, barrier}},
        [:gateway_changed]
      )

    {ready, []} = ProductEventReducer.snapshot_complete(loading)
    ready
  end

  defp event(kind, aggregate_type) do
    %{
      cursor: "cursor-event",
      item:
        {:event,
         %{
           id: "gateway-event",
           cursor: "cursor-event",
           aggregate_type: aggregate_type,
           aggregate_id: "gateway-1",
           aggregate_version: 1,
           payload: {kind, %{}}
         }}
    }
  end
end
