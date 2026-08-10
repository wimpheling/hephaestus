defmodule HephaestusWebWeb.ProjectState do
  @moduledoc "State and effects for the project repositories route."

  alias HephaestusWeb.RPC.{Client, Error, ProductEvents}
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
            data: %{project_id: nil, project: nil, repositories: []},
            form: nil,
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{project_id: project_id}),
    do: %__MODULE__{data: %{project_id: project_id, project: nil, repositories: []}}

  def statuses, do: @statuses
  def stream_mode, do: @stream_mode
  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(state), do: {:project, state.data.project_id}

  def watch(identity, state, owner, generation) do
    ProductEvents.watch(
      identity,
      watch_scope(state),
      ProductEventReducer.committed_cursor(state.cursor),
      fn response ->
        send(owner, {:page_watch, generation, response})

        case response.item do
          {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
          _item -> :cont
        end
      end
    )
  end

  def reduce(state, {:watch, response}),
    do: ProductEventReducer.reduce(state, response, [:project_changed, :repository_changed])

  def reduce(state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(state, {:load, generation}),
    do: {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}

  def reduce(state, {:loaded, generation, project, repositories})
      when generation == state.stream_generation do
    data = %{state.data | project: project, repositories: repositories}
    %{state | data: data} |> ProductEventReducer.snapshot_complete()
  end

  # A snapshot already in flight may finish after a transient watch reconnect.
  # Keep the stale-generation guard for ordinary loads, but do not discard that
  # initial snapshot while the page is recovering its stream.
  def reduce(%{status: :reconnecting} = state, {:loaded, _generation, project, repositories}) do
    data = %{state.data | project: project, repositories: repositories}
    %{state | data: data} |> ProductEventReducer.snapshot_complete()
  end

  def reduce(state, {:loaded, _stale_generation, _project, _repositories}), do: {state, []}

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "Project access was revoked."},
       [{:navigate, :organizations}]}

  def reduce(state, {:error, message}) when is_binary(message),
    do: {%{state | status: :error, error: message}, []}

  def reduce(state, :reconnecting), do: {%{state | status: :reconnecting}, []}
  def reduce(state, :stale), do: {%{state | status: :stale}, [:load]}

  def present(state) do
    Map.merge(state.data, %{
      status: presentation_status(state),
      item_count: length(state.data.repositories),
      error: state.error
    })
  end

  def execute(state, {:load, identity, generation}) do
    project_id = state.data.project_id

    with {:ok, project} <- Client.get_project(identity, project_id),
         {:ok, repositories} <- Client.list_project_repositories(identity, project_id) do
      {:loaded, generation, project, repositories}
    else
      {:error, %Error{kind: kind} = reason} when kind in [:not_found, :permission_denied] ->
        {:access_revoked, reason}

      {:error, %Error{} = reason} ->
        {:error, Error.present(reason)}

      {:error, _reason} ->
        {:error, "The project data is temporarily unavailable."}
    end
  end

  defp presentation_status(%{status: :ready}), do: :ready
  defp presentation_status(%{status: :reconnecting}), do: :reconnecting

  defp presentation_status(%{status: status}) when status in [:initial, :loading, :stale],
    do: :loading

  defp presentation_status(_state), do: :error
end
