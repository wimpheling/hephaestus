defmodule HephaestusWebWeb.ProjectRunsLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectRunsPage
  alias HephaestusWebWeb.PageStream
  alias HephaestusWebWeb.ProjectRunsState
  @stream_mode :page_scoped

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = ProjectRunsState.new(%{project_id: project_id})

    socket =
      socket
      |> stream_configure(:runs, dom_id: &"project-run-#{&1["id"]}")
      |> stream(:runs, [])
      |> assign(:page_state, state)
      |> assign(:watch_task, nil)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Runs")

    if connected?(socket),
      do: {:ok, PageStream.start_watch(socket, ProjectRunsState)},
      else: {:ok, socket}
  end

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_watch(socket, ProjectRunsState, response)
    {:noreply, PageStream.apply_effects(socket, ProjectRunsState, effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_ended(socket, ProjectRunsState, result)
    {:noreply, PageStream.apply_effects(socket, ProjectRunsState, effects)}
  end

  def handle_info({kind, _generation, _value}, socket)
      when kind in [:page_watch, :page_watch_ended],
      do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = ProjectRunsState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> stream(:runs, state.data.runs, reset: true)
     |> PageStream.apply_effects(ProjectRunsState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ),
      do: {:noreply, assign(socket, :snapshot_task, nil)}

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket),
    do:
      (
        PageStream.cancel(socket.assigns[:watch_task])
        PageStream.cancel(socket.assigns[:snapshot_task])
        :ok
      )

  @impl true
  def render(assigns) do
    presentation = ProjectRunsState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <ProjectRunsPage.project_runs_page
        state={@presentation.status}
        project={@presentation.project}
        project_id={@presentation.project_id}
        item_count={@presentation.item_count}
        runs={@streams.runs}
        organization_index_destination={~p"/organizations"}
        organization_destination={organization_destination(@presentation.project)}
        run_destination={fn id -> ~p"/runs/#{id}" end}
      />
    </Layouts.app>
    """
  end

  defp organization_destination(nil), do: "/organizations"
  defp organization_destination(project), do: "/organizations/#{project["organization_id"]}"
end
