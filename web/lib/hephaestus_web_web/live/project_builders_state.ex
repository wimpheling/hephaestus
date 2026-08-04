defmodule HephaestusWebWeb.ProjectBuildersState do
  @moduledoc "State and effects for a project's owned OCI builders."

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
            data: %{project_id: nil, builders: []},
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode
  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(state), do: {:project, state.data.project_id}

  def new(%{project_id: project_id}),
    do: %__MODULE__{data: %{project_id: project_id, builders: []}}

  def watch(identity, state, owner, generation) do
    ProductEvents.watch(
      identity,
      watch_scope(state),
      ProductEventReducer.committed_cursor(state.cursor),
      &deliver_watch(&1, owner, generation)
    )
  end

  def reduce(state, {:watch, response}) do
    ProductEventReducer.reduce(state, response, [:registry_publication_changed])
  end

  def reduce(state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(state, :load) do
    generation = state.stream_generation + 1
    {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}
  end

  def reduce(state, {:loaded, generation, builders}) when generation == state.stream_generation do
    state = %{state | data: %{state.data | builders: builders}, error: nil}
    ProductEventReducer.snapshot_complete(state)
  end

  def reduce(state, {:loaded, _generation, _builders}), do: {state, []}

  def reduce(state, {:failed, _reason}),
    do: {%{state | status: :error, error: "Project builders are unavailable."}, []}

  def reduce(state, :stale), do: {%{state | status: :stale}, [:snapshot]}
  def reduce(state, :reconnecting), do: {%{state | status: :reconnecting}, []}

  def reduce(state, {:access_revoked, _reason}) do
    {%{state | status: :access_revoked, error: "Project access was revoked."},
     [{:navigate, :organizations}]}
  end

  def present(state) do
    %{
      state: presentation_state(state),
      project_id: state.data.project_id,
      builders: state.data.builders,
      item_count: length(state.data.builders),
      error: state.error
    }
  end

  def execute(state, {:load, identity, generation}) do
    case Client.list_project_builders(identity, state.data.project_id) do
      {:ok, builders} -> {:loaded, generation, builders}
      {:error, reason} -> {:failed, reason}
    end
  end

  def execute(state, {:load, identity}),
    do: execute(state, {:load, identity, state.stream_generation})

  defp presentation_state(%{status: :ready}), do: :ready

  defp presentation_state(%{status: status}) when status in [:stale, :reconnecting],
    do: :reconnecting

  defp presentation_state(%{status: :error}), do: :error
  defp presentation_state(_state), do: :loading

  defp deliver_watch(response, owner, generation) do
    send(owner, {:page_watch, generation, response})

    case response.item do
      {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
      _item -> :cont
    end
  end
end
