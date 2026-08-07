defmodule HephaestusWebWeb.ImageCatalogLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.ImageCatalogState
  alias HephaestusWebWeb.DesignSystem.Pages.ImageCatalogPage

  @stream_mode :none

  @impl true
  def mount(_params, _session, socket) do
    _stream_mode = @stream_mode
    state = ImageCatalogState.new(%{})
    {state, _effects} = ImageCatalogState.reduce(state, :load)
    identity = socket.assigns.current_identity

    socket = assign(socket, page_state: state, page_title: "Images")

    if connected?(socket) do
      {:ok,
       start_async(socket, :load, fn -> ImageCatalogState.execute(state, {:load, identity}) end)}
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
    presentation = ImageCatalogState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination="/organizations"
      logout_destination="/logout"
    >
      <ImageCatalogPage.image_catalog_page {@presentation} />
    </Layouts.app>
    """
  end

  defp reduce_event(socket, event) do
    {state, _effects} = ImageCatalogState.reduce(socket.assigns.page_state, event)
    assign(socket, :page_state, state)
  end
end
