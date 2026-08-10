defmodule HephaestusWebWeb.ProductEventReducerTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.ProductEventReducer
  alias HephaestusWebWeb.ProjectState

  test "installs a barrier, snapshots, and advances only accepted event cursors" do
    state = ProjectState.new(%{project_id: "project-1"})

    barrier = %{
      cursor: "cursor-7",
      versions: %{{"project", "project-1"} => 3},
      schema_version: 1
    }

    {loading, [:snapshot]} =
      ProductEventReducer.reduce(
        state,
        %{cursor: "cursor-7", item: {:snapshot_barrier, barrier}},
        [:project_changed]
      )

    assert ProductEventReducer.committed_cursor(loading.cursor) == "cursor-7"
    assert loading.data.watch_snapshot_pending

    {ready, []} = ProductEventReducer.snapshot_complete(loading)

    {changed, [:snapshot]} =
      ProductEventReducer.reduce(
        ready,
        response("event-1", "cursor-8", 4, :project_changed),
        [:project_changed]
      )

    assert ProductEventReducer.committed_cursor(changed.cursor) == "cursor-8"
    assert changed.data.watch_snapshot_pending
  end

  test "keeps an already-rendered page visible while attaching its first watch" do
    state = %{ProjectState.new(%{project_id: "project-1"}) | status: :ready}

    watched = ProductEventReducer.begin_watch(state)

    assert watched.status == :ready
    assert watched.stream_generation == state.stream_generation + 1
  end

  test "suppresses duplicate IDs and versions without repeating snapshot effects" do
    state = barrier_ready_state()

    {changed, [:snapshot]} =
      ProductEventReducer.reduce(
        state,
        response("event-1", "cursor-8", 4, :project_changed),
        [:project_changed]
      )

    {duplicate_id, []} =
      ProductEventReducer.reduce(
        changed,
        response("event-1", "cursor-8-duplicate", 4, :project_changed),
        [:project_changed]
      )

    {duplicate_version, []} =
      ProductEventReducer.reduce(
        duplicate_id,
        response("event-2", "cursor-8-version", 4, :project_changed),
        [:project_changed]
      )

    assert duplicate_version.data.watch_snapshot_pending
    assert ProductEventReducer.committed_cursor(duplicate_version.cursor) == "cursor-8-version"
  end

  test "an event arriving during the barrier snapshot forces a second ordered snapshot" do
    state = ProjectState.new(%{project_id: "project-1"})

    barrier = %{
      cursor: "cursor-7",
      versions: %{{"project", "project-1"} => 3},
      schema_version: 1
    }

    {loading, [:snapshot]} =
      ProductEventReducer.reduce(
        state,
        %{cursor: "cursor-7", item: {:snapshot_barrier, barrier}},
        [:project_changed]
      )

    {buffered, []} =
      ProductEventReducer.reduce(
        loading,
        response("event-1", "cursor-8", 4, :project_changed),
        [:project_changed]
      )

    assert Enum.map(buffered.data.watch_pending_events, & &1.id) == ["event-1"]
    assert ProductEventReducer.committed_cursor(buffered.cursor) == "cursor-8"

    {replaying, [:snapshot]} = ProductEventReducer.snapshot_complete(buffered)
    assert replaying.status == :stale
    assert replaying.data.watch_snapshot_pending
    assert replaying.data.watch_pending_events == []

    {ready, []} = ProductEventReducer.snapshot_complete(replaying)
    assert ready.status == :ready
  end

  test "replaces from a fresh snapshot on aggregate-version or retention gaps" do
    state = barrier_ready_state()

    {version_gap, [:replace_watch]} =
      ProductEventReducer.reduce(
        state,
        response("event-gap", "cursor-10", 5, :project_changed),
        [:project_changed]
      )

    assert version_gap.cursor == nil
    assert version_gap.stream_generation == state.stream_generation + 1

    {retention_gap, [:replace_watch]} =
      ProductEventReducer.reduce(
        state,
        %{cursor: "cursor-12", item: {:retention_gap, %{requested_cursor: "cursor-1"}}},
        [:project_changed]
      )

    assert retention_gap.cursor == nil
    assert retention_gap.stream_generation == state.stream_generation + 1
  end

  test "unrelated variants advance the cursor without visible replacement" do
    state = barrier_ready_state()

    {advanced, []} =
      ProductEventReducer.reduce(
        state,
        response("event-1", "cursor-8", 1, :run_changed, "run", "run-1"),
        [:project_changed]
      )

    assert ProductEventReducer.committed_cursor(advanced.cursor) == "cursor-8"
    refute advanced.data[:watch_snapshot_pending]
  end

  test "access revocation terminates page state without reconnect" do
    state = barrier_ready_state()

    {revoked, [{:navigate, :organizations}]} =
      ProductEventReducer.reduce(
        state,
        %{cursor: "cursor-9", item: {:access_revoked, %{}}},
        [:project_changed]
      )

    assert revoked.status == :access_revoked
    assert ProductEventReducer.reconnect(revoked) == {revoked, []}
  end

  test "waits for a delayed mutation receipt and reconnects from the prior cursor" do
    state = barrier_ready_state()

    receipt = %{
      committed_cursor: "cursor-8",
      event_id: "event-1",
      aggregate_version: 4
    }

    {waiting, []} = ProductEventReducer.await_receipt(state, receipt)
    assert waiting.status == :stale
    assert ProductEventReducer.committed_cursor(waiting.cursor) == "cursor-7"

    {reconnecting, [:replace_watch]} = ProductEventReducer.reconnect(waiting)
    assert ProductEventReducer.committed_cursor(reconnecting.cursor) == "cursor-7"

    {observed, [:snapshot]} =
      ProductEventReducer.reduce(
        reconnecting,
        response("event-1", "cursor-8", 4, :project_changed),
        [:project_changed]
      )

    assert ProductEventReducer.committed_cursor(observed.cursor) == "cursor-8"
    assert observed.data.watch_required_receipts == []
  end

  test "normalizes a projected mutation receipt and refreshes an already observed mutation" do
    assert {:ok, receipt} =
             ProductEventReducer.receipt(%{
               "receipt" => %{
                 "committed_cursor" => "cursor-8",
                 "aggregate_version" => 4,
                 "event_id" => "event-1"
               }
             })

    state = barrier_ready_state()

    {advanced, []} =
      ProductEventReducer.reduce(
        state,
        response("event-1", "cursor-8", 4, :run_changed),
        [:project_changed]
      )

    {refreshing, [:snapshot]} = ProductEventReducer.await_receipt(advanced, receipt)
    assert ProductEventReducer.committed_cursor(refreshing.cursor) == "cursor-8"
  end

  defp barrier_ready_state do
    state = ProjectState.new(%{project_id: "project-1"})

    barrier = %{
      cursor: "cursor-7",
      versions: %{{"project", "project-1"} => 3},
      schema_version: 1
    }

    {loading, [:snapshot]} =
      ProductEventReducer.reduce(
        state,
        %{cursor: "cursor-7", item: {:snapshot_barrier, barrier}},
        [:project_changed]
      )

    {ready, []} = ProductEventReducer.snapshot_complete(loading)
    ready
  end

  defp response(
         id,
         cursor,
         version,
         payload,
         aggregate_type \\ "project",
         aggregate_id \\ "project-1"
       ) do
    %{
      cursor: cursor,
      item:
        {:event,
         %{
           id: id,
           cursor: cursor,
           aggregate_type: aggregate_type,
           aggregate_id: aggregate_id,
           aggregate_version: version,
           payload: {payload, %{}}
         }}
    }
  end
end
