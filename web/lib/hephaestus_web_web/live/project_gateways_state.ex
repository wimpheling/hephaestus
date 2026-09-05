defmodule HephaestusWebWeb.ProjectGatewaysState do
  @moduledoc "State for the authorized project gateway collection."

  alias HephaestusWeb.RPC.Client

  @stream_mode :none
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

  def reduce(state, {:load, generation}),
    do: {%{state | status: :loading, error: nil, stream_generation: generation}, [:load]}

  def reduce(state, {:access_revoked, _reason}),
    do:
      {%{state | status: :access_revoked, error: "Project access was revoked."},
       [{:navigate, :organizations}]}

  def reduce(state, {:loaded, generation, gateways}) when generation == state.stream_generation do
    {%{state | status: :ready, data: %{state.data | gateways: gateways}, error: nil}, []}
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
end
