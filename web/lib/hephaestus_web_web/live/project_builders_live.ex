defmodule HephaestusWebWeb.ProjectBuildersLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectBuildersPage
  alias HephaestusWebWeb.PageStream
  alias HephaestusWebWeb.ProjectBuildersState

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    state = ProjectBuildersState.new(%{project_id: project_id})

    socket =
      socket
      |> assign(:page_state, state)
      |> assign(:page_title, "Project builders")
      |> assign(:snapshot_task, nil)

    if connected?(socket) do
      {state, [:load]} = ProjectBuildersState.reduce(socket.assigns.page_state, :load)

      {:ok,
       socket
       |> assign(:page_state, state)
       |> PageStream.start_snapshot(ProjectBuildersState)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = ProjectBuildersState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> PageStream.apply_effects(ProjectBuildersState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    {state, effects} = ProjectBuildersState.reduce(socket.assigns.page_state, {:failed, reason})

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> PageStream.apply_effects(ProjectBuildersState, effects)}
  end

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket) do
    PageStream.cancel(socket.assigns[:snapshot_task])
    :ok
  end

  @impl true
  def render(assigns) do
    presentation = ProjectBuildersState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination="/organizations"
      logout_destination="/logout"
    >
      <ProjectBuildersPage.project_builders_page {@presentation} />
    </Layouts.app>
    """
  end
end
