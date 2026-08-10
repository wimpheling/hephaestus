defmodule HephaestusWebWeb.ProjectGatewaysLive do
  @moduledoc "Route adapter for the authorized project gateway overview."

  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectGatewaysPage
  alias HephaestusWebWeb.{PageStream, ProjectGatewaysState}

  @stream_mode :page_scoped

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = ProjectGatewaysState.new(%{project_id: project_id})

    socket =
      socket
      |> assign(:project_id, project_id)
      |> assign(:page_state, state)
      |> assign(:watch_task, nil)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Gateways")

    if connected?(socket) do
      {:ok,
       socket
       |> PageStream.start_watch(ProjectGatewaysState)
       |> PageStream.start_snapshot(ProjectGatewaysState)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_watch(socket, ProjectGatewaysState, response)
    {:noreply, PageStream.apply_effects(socket, ProjectGatewaysState, effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_ended(socket, ProjectGatewaysState, result)
    {:noreply, PageStream.apply_effects(socket, ProjectGatewaysState, effects)}
  end

  def handle_info({:page_watch, _generation, _response}, socket), do: {:noreply, socket}
  def handle_info({:page_watch_ended, _generation, _result}, socket), do: {:noreply, socket}

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
  def terminate(_reason, socket) do
    PageStream.cancel(socket.assigns[:watch_task])
    PageStream.cancel(socket.assigns[:snapshot_task])
    :ok
  end

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
