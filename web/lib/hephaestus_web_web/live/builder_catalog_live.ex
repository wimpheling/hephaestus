defmodule HephaestusWebWeb.BuilderCatalogLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.BuilderCatalogState
  alias HephaestusWebWeb.DesignSystem.Pages.BuilderCatalogPage

  @stream_mode :none

  @impl true
  def mount(_params, _session, socket) do
    _stream_mode = @stream_mode
    state = BuilderCatalogState.new(%{})
    {state, _effects} = BuilderCatalogState.reduce(state, :load)
    identity = socket.assigns.current_identity

    socket = assign(socket, page_state: state, page_title: "Builder catalog")

    if connected?(socket) do
      {:ok,
       start_async(socket, :load, fn -> BuilderCatalogState.execute(state, {:load, identity}) end)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_async(:load, {:ok, event}, socket), do: {:noreply, reduce_event(socket, event)}

  def handle_async(:load, {:exit, reason}, socket),
    do: {:noreply, reduce_event(socket, {:failed, reason})}

  @impl true
  def render(assigns) do
    presentation = BuilderCatalogState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination="/organizations"
      logout_destination="/logout"
    >
      <BuilderCatalogPage.builder_catalog_page {@presentation} />
    </Layouts.app>
    """
  end

  defp reduce_event(socket, event) do
    {state, _effects} = BuilderCatalogState.reduce(socket.assigns.page_state, event)
    assign(socket, :page_state, state)
  end
end
