defmodule HephaestusWebWeb.ProductEventReducer do
  @moduledoc """
  Pure cursor, duplicate, version-gap, and lifecycle reduction for page watches.

  Page state remains authoritative in the LiveView socket. This helper only
  updates the standard cursor field and returns declarative effects.
  """

  @seen_event_limit 256
  @pending_event_limit 256
  @required_receipt_limit 16

  @type effect :: :snapshot | :replace_watch | {:navigate, :organizations}

  @type receipt :: %{
          committed_cursor: String.t(),
          event_id: String.t(),
          aggregate_version: pos_integer()
        }

  @doc "Validates and normalizes the receipt returned by every mutation RPC."
  @spec receipt(map()) :: {:ok, receipt()} | {:error, :missing_mutation_receipt}
  def receipt(%{"receipt" => receipt}), do: receipt(receipt)

  def receipt(%{
        "committed_cursor" => committed_cursor,
        "event_id" => event_id,
        "aggregate_version" => aggregate_version
      })
      when is_binary(committed_cursor) and committed_cursor != "" and is_binary(event_id) and
             event_id != "" and is_integer(aggregate_version) and aggregate_version > 0 do
    {:ok,
     %{
       committed_cursor: committed_cursor,
       event_id: event_id,
       aggregate_version: aggregate_version
     }}
  end

  def receipt(_response), do: {:error, :missing_mutation_receipt}

  @doc "Waits for a watch to observe a mutation receipt without advancing its resume cursor."
  @spec await_receipt(struct(), receipt()) :: {struct(), [effect()]}
  def await_receipt(state, receipt) do
    cursor = normalize_cursor(state.cursor)

    if receipt_observed?(cursor, receipt.committed_cursor, receipt.event_id) do
      ensure_snapshot(state, [])
    else
      required = state.data[:watch_required_receipts] || []
      required = Enum.take(required ++ [receipt], -@required_receipt_limit)
      data = Map.put(state.data, :watch_required_receipts, required)
      {%{state | status: stale_status(state), data: data}, []}
    end
  end

  @doc "Returns the opaque cursor value used to resume a generated watch request."
  @spec committed_cursor(term()) :: String.t() | nil
  def committed_cursor(%{committed: committed}), do: committed
  def committed_cursor(_cursor), do: nil

  @doc "Begins a replacement watch generation while preserving its exact resume cursor."
  @spec begin_watch(struct()) :: struct()
  def begin_watch(%{status: :ready, cursor: nil} = state),
    do: %{state | stream_generation: state.stream_generation + 1}

  def begin_watch(state) do
    status = if state.cursor, do: :reconnecting, else: :loading
    %{state | status: status, stream_generation: state.stream_generation + 1}
  end

  @doc "Reduces one normalized watch response for a page's relevant payload variants."
  @spec reduce(struct(), map(), [atom()]) :: {struct(), [effect()]}
  def reduce(state, %{item: {:snapshot_barrier, barrier}}, _relevant_variants) do
    cursor = %{
      committed: barrier.cursor,
      seen_event_ids: [],
      versions: barrier.versions
    }

    {state, _released?} = release_receipts(%{state | cursor: cursor}, barrier.cursor, nil)
    state = %{state | data: Map.put(state.data, :watch_pending_events, [])}
    {mark_snapshot_pending(state), [:snapshot]}
  end

  def reduce(state, %{cursor: committed, item: {:event, event}}, relevant_variants) do
    cursor = normalize_cursor(state.cursor)
    key = {event.aggregate_type, event.aggregate_id}
    previous_version = Map.get(cursor.versions, key, 0)

    cond do
      event.id in cursor.seen_event_ids or event.aggregate_version <= previous_version ->
        state = %{state | cursor: %{cursor | committed: committed || event.cursor}}
        {state, released?} = release_receipts(state, committed || event.cursor, event.id)
        if released?, do: ensure_snapshot(state, []), else: {state, []}

      event.aggregate_version > previous_version + 1 ->
        replacement = %{state | status: stale_status(state), cursor: nil}
        {%{replacement | stream_generation: state.stream_generation + 1}, [:replace_watch]}

      true ->
        cursor = %{
          cursor
          | committed: committed || event.cursor,
            seen_event_ids: remember(cursor.seen_event_ids, event.id),
            versions: Map.put(cursor.versions, key, event.aggregate_version)
        }

        state = %{state | cursor: cursor}
        {state, released?} = release_receipts(state, committed || event.cursor, event.id)
        {payload_kind, _payload} = event.payload

        {state, effects} =
          reduce_relevant_event(state, event, payload_kind in relevant_variants)

        if released?, do: ensure_snapshot(state, effects), else: {state, effects}
    end
  end

  def reduce(state, %{item: {:retention_gap, _gap}}, _relevant_variants) do
    replacement = %{state | status: stale_status(state), cursor: nil}
    {%{replacement | stream_generation: state.stream_generation + 1}, [:replace_watch]}
  end

  def reduce(state, %{item: {:access_revoked, _revoked}}, _relevant_variants) do
    {%{state | status: :access_revoked, error: "Access to this resource was revoked."},
     [{:navigate, :organizations}]}
  end

  @doc "Accepts a snapshot and coalesces events that arrived while it was in flight."
  @spec snapshot_complete(struct()) :: {struct(), [effect()]}
  def snapshot_complete(state) do
    pending_events = state.data[:watch_pending_events] || []

    data =
      state.data
      |> Map.delete(:watch_snapshot_pending)
      |> Map.put(:watch_pending_events, [])

    if pending_events == [] do
      {%{state | status: :ready, data: data, error: nil}, []}
    else
      state = %{state | status: :stale, data: Map.put(data, :watch_snapshot_pending, true)}
      {state, [:snapshot]}
    end
  end

  @doc "Transitions a terminated watch to an exact-cursor replacement generation."
  @spec reconnect(struct()) :: {struct(), [effect()]}
  def reconnect(%{status: :access_revoked} = state), do: {state, []}

  def reconnect(%{status: :ready} = state) do
    state = %{state | stream_generation: state.stream_generation + 1}
    {state, [:replace_watch]}
  end

  def reconnect(state) do
    state = %{state | status: :reconnecting, stream_generation: state.stream_generation + 1}
    {state, [:replace_watch]}
  end

  @doc "Maps a scoped watch denial to revocation and reconnects transient endings."
  @spec watch_ended(struct(), term()) :: {struct(), [effect()]}
  def watch_ended(state, {:error, %HephaestusWeb.RPC.Error{kind: kind}})
      when kind in [:not_found, :permission_denied, :unauthenticated] do
    {%{state | status: :access_revoked, error: "Access to this resource was revoked."},
     [{:navigate, :organizations}]}
  end

  def watch_ended(state, _result), do: reconnect(state)

  defp normalize_cursor(%{committed: _, seen_event_ids: _, versions: _} = cursor), do: cursor

  defp normalize_cursor(_cursor),
    do: %{committed: nil, seen_event_ids: [], versions: %{}}

  defp remember(ids, id), do: Enum.take([id | ids], @seen_event_limit)
  defp snapshot_pending?(state), do: state.data[:watch_snapshot_pending] == true

  defp mark_snapshot_pending(state) do
    %{
      state
      | status: stale_status(state),
        data: Map.put(state.data, :watch_snapshot_pending, true)
    }
  end

  defp reduce_relevant_event(state, _event, false), do: {state, []}

  defp reduce_relevant_event(state, event, true) do
    if snapshot_pending?(state) do
      pending_events = state.data[:watch_pending_events] || []

      if length(pending_events) >= @pending_event_limit do
        replacement = %{state | status: stale_status(state), cursor: nil}
        {%{replacement | stream_generation: state.stream_generation + 1}, [:replace_watch]}
      else
        data = Map.put(state.data, :watch_pending_events, pending_events ++ [event])
        {%{state | data: data}, []}
      end
    else
      state = %{state | data: Map.put(state.data, :watch_pending_events, [])}
      {mark_snapshot_pending(state), [:snapshot]}
    end
  end

  defp release_receipts(state, committed_cursor, event_id) do
    required = state.data[:watch_required_receipts] || []

    {released, waiting} =
      Enum.split_with(required, fn receipt ->
        receipt.committed_cursor == committed_cursor or receipt.event_id == event_id
      end)

    data = Map.put(state.data, :watch_required_receipts, waiting)
    {%{state | data: data}, released != []}
  end

  defp receipt_observed?(cursor, committed_cursor, event_id) do
    cursor.committed == committed_cursor or event_id in cursor.seen_event_ids
  end

  defp ensure_snapshot(state, effects) do
    if :snapshot in effects or snapshot_pending?(state) do
      {state, effects}
    else
      state = %{state | data: Map.put(state.data, :watch_pending_events, [])}
      {mark_snapshot_pending(state), effects ++ [:snapshot]}
    end
  end

  defp stale_status(%{status: status}) when status in [:initial, :loading], do: :loading
  defp stale_status(_state), do: :stale
end
