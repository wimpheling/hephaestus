defmodule Fixture.DashboardState do
  alias Fixture.Generated.DashboardClient

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
            data: nil,
            form: %{},
            error: nil,
            cursor: nil,
            stream_generation: 0

  def statuses, do: @statuses
  def new(_params), do: %__MODULE__{data: %{label: "Dashboard"}}
  def reduce(state, :load), do: {%{state | status: :loading}, [:load]}
  def present(state), do: %{status: state.status, label: state.data.label}
  def execute(:load, client), do: DashboardClient.list_dashboards(client)
end
