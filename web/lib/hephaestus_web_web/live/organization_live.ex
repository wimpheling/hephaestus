defmodule HephaestusWebWeb.OrganizationLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.OrganizationPage
  alias HephaestusWebWeb.{OrganizationState, PageStream}

  @stream_mode :page_scoped

  @impl true
  def mount(_params, _session, socket) do
    _stream_mode = @stream_mode
    state = OrganizationState.new(%{})

    socket =
      socket
      |> stream_configure(:organizations, dom_id: &"organization-#{&1["id"]}")
      |> stream(:organizations, [])
      |> assign(:page_state, state)
      |> assign(:watch_task, nil)
      |> assign(:snapshot_task, nil)
      |> assign(:stream_mode, @stream_mode)
      |> assign(:page_title, "Organizations")

    if connected?(socket) do
      {:ok, PageStream.start_watch(socket, OrganizationState)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_watch(socket, OrganizationState, response)
    {:noreply, socket |> sync_state() |> PageStream.apply_effects(OrganizationState, effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_ended(socket, OrganizationState, result)
    {:noreply, socket |> sync_state() |> PageStream.apply_effects(OrganizationState, effects)}
  end

  def handle_info({:page_watch, _generation, _response}, socket), do: {:noreply, socket}
  def handle_info({:page_watch_ended, _generation, _result}, socket), do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = OrganizationState.reduce(socket.assigns.page_state, event)

    socket =
      socket
      |> assign(:snapshot_task, nil)
      |> assign(:page_state, state)
      |> sync_state()
      |> PageStream.apply_effects(OrganizationState, effects)

    {:noreply, socket}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    {state, _effects} = OrganizationState.reduce(socket.assigns.page_state, {:failed, reason})
    {:noreply, socket |> assign(:snapshot_task, nil) |> assign(:page_state, state)}
  end

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket) do
    PageStream.cancel(socket.assigns[:watch_task])
    PageStream.cancel(socket.assigns[:snapshot_task])
    :ok
  end

  @impl true
  def render(assigns) do
    presentation = OrganizationState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <OrganizationPage.organization_page
        state={@presentation.status}
        current_identity={@current_identity}
        organization_count={@presentation.organization_count}
        organizations={@streams.organizations}
      />
    </Layouts.app>
    """
  end

  defp sync_state(socket) do
    organizations = socket.assigns.page_state.data.organizations
    stream(socket, :organizations, organizations, reset: true)
  end
end
