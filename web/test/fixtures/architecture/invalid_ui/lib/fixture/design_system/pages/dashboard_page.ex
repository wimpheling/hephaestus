defmodule Fixture.DesignSystem.Pages.DashboardPage do
  use Phoenix.Component

  alias Fixture.DashboardService
  alias Fixture.Generated.DashboardClient
  alias Heroicons.Outline

  @states [:loading, :ready]

  attr :on_retry, :map

  def page(assigns) do
    ~H"""
    <%!-- <article> inside a comment must not be reported. --%>
    <main data-name={@name}>
      <input name="name" />
      <button type="button">Save</button>
      <svg viewBox="0 0 10 10" />
      <.button class="page-authored" phx-click="save" phx-value-kind={"literal" <> ""}>
        Save through component
      </.button>
    </main>
    """
  end

  def handle_event(_event, _params, socket), do: {:noreply, socket}
  def client, do: DashboardClient
  def service, do: DashboardService
  def external_icon, do: Outline
end
