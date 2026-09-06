defmodule HephaestusWebWeb.ProjectRepositoryImageState do
  @moduledoc "Redacted repository-image detail backed by a reauthorized project watch."

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
            data: %{project_id: nil, image_id: nil, image: nil, history: []},
            form: nil,
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{project_id: project_id, image_id: image_id}),
    do: %__MODULE__{data: %{project_id: project_id, image_id: image_id, image: nil, history: []}}

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
    do: ProductEventReducer.reduce(state, response, [:project_changed])

  def reduce(state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(state, {:loaded, generation, image}) when generation == state.stream_generation do
    data = %{state.data | image: image["image"], history: image["history"] || []}
    %{state | data: data} |> ProductEventReducer.snapshot_complete()
  end

  def reduce(state, {:loaded, _generation, _image}), do: {state, []}

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "Project image access was revoked."},
       [{:navigate, :organizations}]}

  def reduce(state, :load_failed),
    do: {%{state | status: :error, error: "Image detail is unavailable."}, []}

  def execute(state, {:load, identity, generation}) do
    case Client.get_project_repository_image(identity, state.data.image_id) do
      {:ok, image} -> {:loaded, generation, image}
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  def present(state),
    do: Map.merge(state.data, %{status: page_status(state.status), error: state.error})

  defp page_status(:ready), do: :ready
  defp page_status(:reconnecting), do: :reconnecting
  defp page_status(status) when status in [:error, :access_revoked], do: :error
  defp page_status(_status), do: :loading

  defp deliver_watch(response, owner, generation) do
    send(owner, {:page_watch, generation, response})

    case response.item do
      {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
      _item -> :cont
    end
  end
end
