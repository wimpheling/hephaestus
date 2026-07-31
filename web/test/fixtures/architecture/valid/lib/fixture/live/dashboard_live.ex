defmodule Fixture.DashboardLive do
  use Phoenix.LiveView

  alias Fixture.DashboardState
  alias Fixture.DesignSystem.Pages.DashboardPage

  @stream_mode :none

  def mount(_params, _session, socket) do
    {:ok, assign(socket, :page_state, DashboardState.new(%{}))}
  end

  def render(assigns), do: ~H"<DashboardPage.page label={@page_state.data.label} />"
end
