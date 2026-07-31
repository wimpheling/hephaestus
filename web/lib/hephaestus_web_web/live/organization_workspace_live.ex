defmodule HephaestusWebWeb.OrganizationWorkspaceLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.OrganizationWorkspacePage
  alias HephaestusWebWeb.OrganizationWorkspaceState
  alias HephaestusWebWeb.PageStream

  @stream_mode :page_scoped

  @impl true
  def mount(%{"organization_id" => organization_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = OrganizationWorkspaceState.new(%{organization_id: organization_id})

    socket =
      socket
      |> assign(:page_state, state)
      |> assign(:watch_task, nil)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Projects")

    if connected?(socket),
      do: {:ok, PageStream.start_watch(socket, OrganizationWorkspaceState)},
      else: {:ok, socket}
  end

  @impl true
  def handle_params(_params, _uri, socket), do: {:noreply, socket}

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_watch(socket, OrganizationWorkspaceState, response)
    {:noreply, PageStream.apply_effects(socket, OrganizationWorkspaceState, effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_ended(socket, OrganizationWorkspaceState, result)
    {:noreply, PageStream.apply_effects(socket, OrganizationWorkspaceState, effects)}
  end

  def handle_info({kind, _generation, _value}, socket)
      when kind in [:page_watch, :page_watch_ended],
      do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = OrganizationWorkspaceState.reduce(socket.assigns.page_state, event)
    socket = socket |> assign(:page_state, state) |> assign(:snapshot_task, nil)

    {:noreply,
     socket
     |> apply_state_effects(effects)
     |> PageStream.apply_effects(OrganizationWorkspaceState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ),
      do: {:noreply, assign(socket, :snapshot_task, nil)}

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket) do
    PageStream.cancel(socket.assigns[:watch_task])
    PageStream.cancel(socket.assigns[:snapshot_task])
    :ok
  end

  @impl true
  def render(assigns) do
    presentation = OrganizationWorkspaceState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <OrganizationWorkspacePage.organization_workspace_page
        state={@presentation.status}
        organization={@presentation.organization}
        projects={@presentation.projects}
      />
    </Layouts.app>
    """
  end

  defp apply_state_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
      {:navigate, :organizations}, socket ->
        socket
        |> put_flash(:error, "Organization access was revoked.")
        |> push_navigate(to: ~p"/organizations")

      _effect, socket ->
        socket
    end)
  end
end
