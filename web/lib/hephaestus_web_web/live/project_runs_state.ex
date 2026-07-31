defmodule HephaestusWebWeb.ProjectRunsState do
  @moduledoc "State and effects for the project runs route."

  alias HephaestusWeb.RPC.{Client, ProductEvents}
  alias HephaestusWebWeb.ProductEventReducer

  @stream_mode :page_scoped
  @statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]

  defstruct status: :initial,
            data: %{project_id: nil, project: nil, runs: []},
            form: nil,
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{project_id: project_id}),
    do: %__MODULE__{data: %{project_id: project_id, project: nil, runs: []}}

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode
  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(state), do: {:project, state.data.project_id}

  def watch(identity, state, owner, generation) do
    ProductEvents.watch(
      identity,
      watch_scope(state),
      ProductEventReducer.committed_cursor(state.cursor),
      &deliver_watch(&1, owner, generation)
    )
  end

  def reduce(state, {:watch, response}),
    do: ProductEventReducer.reduce(state, response, [:project_changed, :run_changed])

  def reduce(state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(state, {:load, generation}),
    do: {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}

  def reduce(state, {:loaded, generation, project, runs})
      when generation == state.stream_generation do
    state = %{state | data: %{state.data | project: project, runs: runs}}
    ProductEventReducer.snapshot_complete(state)
  end

  def reduce(state, {:loaded, _generation, _project, _runs}), do: {state, []}

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "Project access was revoked."},
       [{:navigate, :organizations}]}

  def reduce(state, :reconnecting), do: {%{state | status: :reconnecting}, []}
  def reduce(state, :stale), do: {%{state | status: :stale}, [:load]}

  def present(state),
    do:
      Map.merge(state.data, %{
        status: presentation_status(state),
        item_count: length(state.data.runs),
        error: state.error
      })

  def execute(state, {:load, identity, generation}) do
    project_id = state.data.project_id

    with {:ok, project} <- Client.get_project(identity, project_id),
         {:ok, runs} <- Client.list_project_runs(identity, project_id) do
      {:loaded, generation, project, runs}
    else
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  defp presentation_status(%{status: :ready}), do: :ready
  defp presentation_status(%{status: :reconnecting}), do: :reconnecting

  defp presentation_status(%{status: status}) when status in [:initial, :loading, :stale],
    do: :loading

  defp presentation_status(_state), do: :error

  defp deliver_watch(response, owner, generation) do
    send(owner, {:page_watch, generation, response})

    case response.item do
      {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
      _item -> :cont
    end
  end
end
