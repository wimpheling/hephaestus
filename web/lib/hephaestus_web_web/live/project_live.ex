defmodule HephaestusWebWeb.ProjectLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectPage
  alias HephaestusWebWeb.ProjectState

  @stream_mode :page_scoped

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = ProjectState.new(%{project_id: project_id})

    socket =
      socket
      |> stream_configure(:repositories, dom_id: &"project-repository-#{&1["id"]}")
      |> stream(:repositories, [])
      |> assign(:page_state, state)
      |> assign(:watch_task, nil)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Repositories")

    if connected?(socket) do
      socket = start_watch(socket)
      {:ok, start_snapshot(socket)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_params(_params, _uri, socket), do: {:noreply, socket}

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {state, effects} = ProjectState.reduce(socket.assigns.page_state, {:watch, response})
    {:noreply, socket |> sync_state(state) |> apply_watch_effects(effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, _result},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {state, effects} = ProjectState.reduce(socket.assigns.page_state, :watch_ended)

    {:noreply,
     socket |> assign(:watch_task, nil) |> sync_state(state) |> apply_watch_effects(effects)}
  end

  def handle_info({:page_watch, _stale_generation, _response}, socket), do: {:noreply, socket}
  def handle_info({:page_watch_ended, _stale_generation, _result}, socket), do: {:noreply, socket}

  def handle_info(
        {ref, event},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    Process.demonitor(ref, [:flush])
    {state, effects} = ProjectState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:snapshot_task, nil)
     |> sync_state(state)
     |> stream(:repositories, state.data.repositories, reset: true)
     |> apply_watch_effects(effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    {:noreply, assign(socket, :snapshot_task, nil)}
  end

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket),
    do:
      (
        cancel_task(socket.assigns[:watch_task])
        cancel_task(socket.assigns[:snapshot_task])
        :ok
      )

  @impl true
  def render(assigns) do
    presentation = ProjectState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <ProjectPage.project_page
        state={@presentation.status}
        project={@presentation.project}
        project_id={@presentation.project_id}
        item_count={@presentation.item_count}
        repositories={@streams.repositories}
        organization_index_destination={~p"/organizations"}
        organization_destination={organization_destination(@presentation.project)}
        repository_destination={fn id -> ~p"/repositories/#{id}" end}
      />
    </Layouts.app>
    """
  end

  defp start_watch(socket, increment? \\ true) do
    cancel_task(socket.assigns[:watch_task])
    state = socket.assigns.page_state
    state = if increment?, do: ProjectState.begin_watch(state), else: state
    identity = socket.assigns.current_identity
    generation = state.stream_generation
    # Resume from the last committed backend cursor, never from rendered UI state.
    _committed_cursor = state.cursor
    owner = self()

    {:ok, task} =
      Task.Supervisor.start_child(HephaestusWeb.PageTaskSupervisor, fn ->
        result = ProjectState.watch(identity, state, owner, generation)
        send(owner, {:page_watch_ended, generation, result})
      end)

    socket |> sync_state(state) |> assign(:watch_task, task)
  end

  defp start_snapshot(socket) do
    cancel_task(socket.assigns[:snapshot_task])
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    generation = state.stream_generation

    task =
      Task.Supervisor.async_nolink(HephaestusWeb.PageTaskSupervisor, fn ->
        ProjectState.execute(state, {:load, identity, generation})
      end)

    assign(socket, :snapshot_task, task)
  end

  defp apply_watch_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
      :snapshot, socket -> start_snapshot(socket)
      :replace_watch, socket -> start_watch(socket, false)
      {:navigate, :organizations}, socket -> maybe_revoke_access(socket)
    end)
  end

  defp sync_state(socket, state), do: assign(socket, :page_state, state)

  defp cancel_task(nil), do: :ok
  defp cancel_task(%Task{pid: pid}), do: cancel_task(pid)

  defp cancel_task(pid) when is_pid(pid),
    do: Task.Supervisor.terminate_child(HephaestusWeb.PageTaskSupervisor, pid)

  defp organization_destination(nil), do: "/organizations"
  defp organization_destination(project), do: "/organizations/#{project["organization_id"]}"

  defp maybe_revoke_access(%{assigns: %{page_state: %{status: :access_revoked}}} = socket),
    do:
      socket
      |> put_flash(:error, "That project is not visible.")
      |> push_navigate(to: ~p"/organizations")

  defp maybe_revoke_access(socket), do: socket
end
