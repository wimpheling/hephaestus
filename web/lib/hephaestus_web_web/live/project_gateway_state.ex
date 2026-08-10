defmodule HephaestusWebWeb.ProjectGatewayState do
  @moduledoc "Gateway detail state backed by a reauthorized project event watch."

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
            data: %{project_id: nil, gateway_id: nil, gateway: nil, ingress: []},
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def new(%{project_id: project_id, gateway_id: gateway_id}),
    do: %__MODULE__{
      data: %{project_id: project_id, gateway_id: gateway_id, gateway: nil, ingress: []}
    }

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
      {%{state | status: :access_revoked, error: "Gateway access was revoked."},
       [{:navigate, :organizations}]}

  def reduce(state, {:loaded, generation, gateway, ingress})
      when generation == state.stream_generation do
    %{state | data: %{state.data | gateway: gateway, ingress: ingress}}
    |> ProductEventReducer.snapshot_complete()
  end

  def reduce(state, {:loaded, _generation, _gateway, _ingress}), do: {state, []}

  def reduce(state, {:transitioned, response}) do
    gateway = response["gateway"] || state.data.gateway
    state = %{state | data: %{state.data | gateway: gateway}}

    case ProductEventReducer.receipt(response) do
      {:ok, receipt} -> ProductEventReducer.await_receipt(state, receipt)
      {:error, _reason} -> {%{state | status: :stale}, [:snapshot]}
    end
  end

  def reduce(state, {:lifecycle, next}) do
    {%{state | status: :submitting, error: nil}, [{:lifecycle, next}]}
  end

  def reduce(state, :lifecycle_failed) do
    {%{state | status: :error, error: "Lifecycle update was not accepted."}, []}
  end

  def execute(state, {:load, identity, generation}) do
    with {:ok, gateway} <- Client.get_gateway(identity, state.data.gateway_id),
         {:ok, ingress} <- Client.list_gateway_ingress(identity, state.data.gateway_id) do
      {:loaded, generation, gateway, ingress}
    else
      {:error, reason} -> {:access_revoked, reason}
    end
  end

  def execute(state, {:lifecycle, identity, next}) do
    gateway = state.data.gateway
    Client.set_gateway_lifecycle(identity, gateway["id"], gateway["lifecycle"], next)
  end

  def present(state),
    do: %{
      status: page_status(state.status),
      gateway: state.data.gateway,
      ingress: state.data.ingress,
      lifecycle_actions: lifecycle_actions(state.data.gateway),
      error: state.error
    }

  defp lifecycle_actions(nil), do: []

  defp lifecycle_actions(%{"lifecycle" => "enabled"}),
    do: [
      %{label: "Pause", next: "paused", variant: :secondary},
      %{label: "Remove", next: "removed", variant: :danger}
    ]

  defp lifecycle_actions(%{"lifecycle" => "paused"}),
    do: [
      %{label: "Recover", next: "enabled", variant: :primary},
      %{label: "Remove", next: "removed", variant: :danger}
    ]

  defp lifecycle_actions(%{"lifecycle" => "removed"}), do: []

  defp lifecycle_actions(_gateway),
    do: [%{label: "Remove", next: "removed", variant: :danger}]

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
