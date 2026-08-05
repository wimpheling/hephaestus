defmodule HephaestusWebWeb.ProjectAgentsLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectAgentsPage
  alias HephaestusWebWeb.PageStream
  alias HephaestusWebWeb.ProjectAgentsState

  @impl true
  def mount(%{"project_id" => project_id}, _session, socket) do
    state = ProjectAgentsState.new(%{project_id: project_id})

    socket =
      socket
      |> stream_configure(:instances, dom_id: &"project-instance-#{&1["id"]}")
      |> stream(:instances, [])
      |> assign(:page_state, state)
      |> assign(:snapshot_task, nil)
      |> assign(:page_title, "Agents")

    if connected?(socket) do
      {state, [:load]} = ProjectAgentsState.reduce(socket.assigns.page_state, {:load, 1})

      {:ok,
       socket
       |> assign(:page_state, state)
       |> PageStream.start_snapshot(ProjectAgentsState)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = ProjectAgentsState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:page_state, state)
     |> assign(:snapshot_task, nil)
     |> stream(:instances, state.data.instances, reset: true)
     |> PageStream.apply_effects(ProjectAgentsState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ),
      do: {:noreply, assign(socket, :snapshot_task, nil)}

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def handle_event("import-agent", %{"import" => attributes}, socket) do
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    {submitting, _effects} = ProjectAgentsState.reduce(state, :submitting)

    {:noreply,
     socket
     |> assign(:page_state, submitting)
     |> start_async(:command, fn ->
       ProjectAgentsState.execute_sensitive(state, identity, :import, attributes)
     end)}
  end

  @impl true
  def handle_async(:command, {:ok, {:command_succeeded, message, instance_id}}, socket),
    do:
      {:noreply,
       socket
       |> put_flash(:info, message)
       |> push_navigate(
         to: "/projects/#{socket.assigns.page_state.data.project_id}/agents/#{instance_id}"
       )}

  def handle_async(:command, {:ok, {:failed, _reason}}, socket),
    do: {:noreply, put_flash(socket, :error, "Import could not be completed.")}

  def handle_async(:command, {:exit, _reason}, socket),
    do: {:noreply, put_flash(socket, :error, "Import could not be completed.")}

  @impl true
  def terminate(_reason, socket),
    do:
      (
        PageStream.cancel(socket.assigns[:snapshot_task])
        :ok
      )

  @impl true
  def render(assigns) do
    presentation = ProjectAgentsState.present(assigns.page_state)
    assigns = assign(assigns, :presentation, presentation)

    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination={~p"/organizations"}
      logout_destination={~p"/logout"}
    >
      <ProjectAgentsPage.project_agents_page
        state={@presentation.status}
        project={@presentation.project}
        project_id={@presentation.project_id}
        item_count={@presentation.item_count}
        instances={@streams.instances}
        release_catalog={@presentation.release_catalog}
        form={@presentation.import_form}
        organization_index_destination={~p"/organizations"}
        organization_destination={organization_destination(@presentation.project)}
        instance_destination={fn id -> "/projects/#{@presentation.project_id}/agents/#{id}" end}
        import_event="import-agent"
      />
    </Layouts.app>
    """
  end

  defp organization_destination(nil), do: "/organizations"
  defp organization_destination(project), do: "/organizations/#{project["organization_id"]}"
end
