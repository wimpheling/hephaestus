defmodule HephaestusWebWeb.ProjectGatewaysState do
  @moduledoc "State and reauthorized project-event watch for gateway browsing."

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
            data: %{project_id: nil, gateways: []},
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{project_id: project_id}),
    do: %__MODULE__{data: %{project_id: project_id, gateways: []}}

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
    do: ProductEventReducer.reduce(state, response, [:gateway_changed])

  def reduce(state, :watch_ended), do: ProductEventReducer.reconnect(state)

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "Project access was revoked."},
       [{:navigate, :organizations}]}

  def reduce(state, {:loaded, generation, gateways}) when generation == state.stream_generation do
    %{state | data: %{state.data | gateways: gateways}} |> ProductEventReducer.snapshot_complete()
  end

  def reduce(state, {:loaded, _generation, _gateways}), do: {state, []}

  def execute(state, {:load, identity, generation}) do
    case Client.list_project_gateways(identity, state.data.project_id) do
      {:ok, gateways} -> {:loaded, generation, gateways}
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  def present(state),
    do: %{
      status: page_status(state.status),
      gateways: state.data.gateways,
      error: state.error
    }

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
