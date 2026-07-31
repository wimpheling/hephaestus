defmodule Fixture.DashboardLive do
  use Phoenix.LiveView

  alias Fixture.DashboardService
  alias Fixture.DesignSystem.Pages.DashboardPage

  @stream_mode :page_scoped

  def mount(_params, _session, socket) do
    {:ok, assign(socket, :secret_token, "plaintext")}
  end

  def render(assigns) do
    ~H"""
    <DashboardPage.page name={@name} />
    <DashboardPage.page name={@name} />
    """
  end

  def backend, do: DashboardService
  defp runtime, do: Process.put(:dashboard_state, true)
end
