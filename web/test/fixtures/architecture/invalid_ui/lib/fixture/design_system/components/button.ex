defmodule Fixture.DesignSystem.Components.Button do
  use Phoenix.Component

  alias Fixture.DesignSystem.Composites.Panel
  alias Fixture.Generated.DashboardClient
  alias Fixture.DashboardLive
  alias Fixture.Router

  def button(assigns) do
    ~H"""
    <button type="button">{@label}</button>
    """
  end

  def composite_reference, do: Panel
  def generated_reference, do: DashboardClient
  def live_reference, do: DashboardLive
  def route_reference, do: Router
end
