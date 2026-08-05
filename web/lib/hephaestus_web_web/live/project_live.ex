defmodule HephaestusWebWeb.ProjectLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectPage
  alias HephaestusWebWeb.ProjectState

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    state = ProjectState.new(%{project_id: project_id})

    socket =
      socket
      |> stream_configure(:repositories, dom_id: &"project-repository-#{&1["id"]}")
      |> stream(:repositories, [])
      |> assign(:page_state, state)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Repositories")

    if connected?(socket) do
      {:ok, start_snapshot(socket)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_params(_params, _uri, socket), do: {:noreply, socket}

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
     |> apply_state_effects(effects)}
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
      <div id="project-live-root">
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
      </div>
    </Layouts.app>
    """
  end

  defp start_snapshot(socket) do
    cancel_task(socket.assigns[:snapshot_task])
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    generation = state.stream_generation

    # This short snapshot must not queue behind page-owned event watches in the
    # shared task supervisor. Its monitor is owned by this LiveView and is
    # cleaned up by the existing result/DOWN handlers below.
    task = Task.async(fn -> ProjectState.execute(state, {:load, identity, generation}) end)

    assign(socket, :snapshot_task, task)
  end

  defp apply_state_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
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
