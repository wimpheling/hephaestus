defmodule Fixture.DashboardState do
  use Phoenix.Component

  alias Fixture.DesignSystem
  alias Phoenix.LiveView.Socket

  @statuses [:loading, :ready]

  defstruct status: :loading, password: nil

  def leaked_render(assigns), do: ~H"<DesignSystem.button label={@label} />"
  def socket(%Socket{} = socket), do: socket
  def runtime, do: :ets.new(:dashboard_state, [])
end
