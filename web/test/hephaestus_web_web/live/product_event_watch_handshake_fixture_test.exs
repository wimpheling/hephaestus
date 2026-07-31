defmodule HephaestusWebWeb.ProductEventWatchHandshakeFixtureTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.ProductEventReducer
  alias HephaestusWebWeb.ProjectState

  @fixture_path Path.expand("../../fixtures/product_event_watch_handshake.json", __DIR__)

  test "subscribe-first barrier buffers, deduplicates, and replays before becoming ready" do
    fixture = @fixture_path |> File.read!() |> Jason.decode!()
    barrier = fixture["barrier"]
    event = fixture["buffered_event"]
    state = ProjectState.new(%{project_id: barrier["aggregate_id"]})

    barrier_item = %{
      cursor: barrier["cursor"],
      versions: %{
        {barrier["aggregate_type"], barrier["aggregate_id"]} => barrier["aggregate_version"]
      },
      schema_version: barrier["schema_version"]
    }

    {loading, [:snapshot]} =
      ProductEventReducer.reduce(
        state,
        %{cursor: barrier["cursor"], item: {:snapshot_barrier, barrier_item}},
        [:project_changed]
      )

    response = event_response(event)
    {buffered, []} = ProductEventReducer.reduce(loading, response, [:project_changed])
    {deduplicated, []} = ProductEventReducer.reduce(buffered, response, [:project_changed])

    assert Enum.map(deduplicated.data.watch_pending_events, & &1.id) == [event["id"]]
    assert ProductEventReducer.committed_cursor(deduplicated.cursor) == event["cursor"]

    {replaying, [:snapshot]} = ProductEventReducer.snapshot_complete(deduplicated)
    {ready, []} = ProductEventReducer.snapshot_complete(replaying)
    assert ready.status == :ready
  end

  test "retention gaps restart without a cursor and access revocation is terminal" do
    fixture = @fixture_path |> File.read!() |> Jason.decode!()
    state = ProjectState.new(%{project_id: "project-1"})

    {replacement, [:replace_watch]} =
      ProductEventReducer.reduce(
        state,
        %{item: {:retention_gap, fixture["retention_gap"]}},
        [:project_changed]
      )

    assert replacement.cursor == nil

    {revoked, [{:navigate, :organizations}]} =
      ProductEventReducer.reduce(
        replacement,
        %{item: {:access_revoked, %{}}},
        [:project_changed]
      )

    assert ProductEventReducer.reconnect(revoked) == {revoked, []}
  end

  defp event_response(event) do
    payload_kind = String.to_existing_atom(event["payload_kind"])

    %{
      cursor: event["cursor"],
      item:
        {:event,
         %{
           id: event["id"],
           cursor: event["cursor"],
           aggregate_type: event["aggregate_type"],
           aggregate_id: event["aggregate_id"],
           aggregate_version: event["aggregate_version"],
           payload: {payload_kind, %{}}
         }}
    }
  end
end
