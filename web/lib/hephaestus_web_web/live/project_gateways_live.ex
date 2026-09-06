defmodule HephaestusWebWeb.ProjectGatewaysLive do
  @moduledoc "Route adapter for the authorized project gateway overview."

  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectGatewaysPage
  alias HephaestusWebWeb.{PageStream, ProjectGatewaysState}

  # Gateway browsing is a finite, authorized read like the other project tabs.
  # Lifecycle detail owns its own explicit refresh after a mutation.
  @stream_mode :none

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = ProjectGatewaysState.new(%{project_id: project_id})

    socket =
      socket
      |> assign(:project_id, project_id)
      |> assign(:page_state, state)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Gateways")

    if connected?(socket) do
      {state, [:load]} = ProjectGatewaysState.reduce(state, {:load, 1})

      {:ok,
       socket
       |> assign(:page_state, state)
       |> PageStream.start_snapshot(ProjectGatewaysState)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = ProjectGatewaysState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> PageStream.apply_effects(ProjectGatewaysState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ),
      do: {:noreply, assign(socket, :snapshot_task, nil)}

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket), do: PageStream.cancel(socket.assigns[:snapshot_task])

  @impl true
  def render(assigns) do
    presentation = ProjectGatewaysState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <ProjectGatewaysPage.project_gateways_page
        state={@presentation.status}
        project_id={@project_id}
        gateways={@presentation.gateways}
        gateway_destination={fn gateway_id -> ~p"/projects/#{@project_id}/gateways/#{gateway_id}" end}
      />
    </Layouts.app>
    """
  end
end
