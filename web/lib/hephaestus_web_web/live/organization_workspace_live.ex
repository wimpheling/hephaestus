defmodule HephaestusWebWeb.OrganizationWorkspaceLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.OrganizationWorkspacePage
  alias HephaestusWebWeb.OrganizationWorkspaceState
  alias HephaestusWebWeb.PageStream

  @impl true
  def mount(%{"organization_id" => organization_id}, _session, socket) do
    state = OrganizationWorkspaceState.new(%{organization_id: organization_id})

    socket =
      socket
      |> assign(:page_state, state)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Projects")

    if connected?(socket) do
      {state, [:load]} = OrganizationWorkspaceState.reduce(socket.assigns.page_state, :load)

      {:ok,
       socket
       |> assign(:page_state, state)
       |> PageStream.start_snapshot(OrganizationWorkspaceState)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_params(_params, _uri, socket), do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = OrganizationWorkspaceState.reduce(socket.assigns.page_state, event)
    socket = socket |> assign(:page_state, state) |> assign(:snapshot_task, nil)

    {:noreply,
     socket
     |> apply_state_effects(effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ),
      do: {:noreply, assign(socket, :snapshot_task, nil)}

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket) do
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
