defmodule HephaestusWebWeb.AgentInstanceLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.AgentInstanceState
  alias HephaestusWebWeb.DesignSystem.Pages.AgentInstancePage
  alias HephaestusWebWeb.PageStream

  @events [
    "create-attachment",
    "set-attachment",
    "remove-attachment",
    "revise-instance",
    "revise-capabilities",
    "create-update",
    "recover-update",
    "bind-secret"
  ]

  @impl true
  def mount(%{"instance_id" => instance_id}, _session, socket) do
    state = AgentInstanceState.new(instance_id)

    socket =
      socket
      |> stream_configure(:revisions, dom_id: &"instance-revision-#{&1["id"]}")
      |> stream_configure(:attachments, dom_id: &"instance-attachment-#{&1["id"]}")
      |> stream_configure(:updates, dom_id: &"instance-update-#{&1["id"]}")
      |> stream(:revisions, [])
      |> stream(:attachments, [])
      |> stream(:updates, [])
      |> assign(:page_state, state)
      |> assign(:presentation, AgentInstanceState.present(state))
      |> assign(:effect_task, nil)
      |> assign(:snapshot_task, nil)
      |> assign(:cursor, state.cursor)
      |> assign(:stream_generation, state.stream_generation)

    if connected?(socket) do
      {state, [_effect]} = AgentInstanceState.reduce(state, :load)

      {:ok,
       socket
       |> sync_state(state)
       |> PageStream.start_snapshot(AgentInstanceState)}
    else
      {:ok, socket}
    end
  end

  @impl true
  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = AgentInstanceState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:snapshot_task, nil)
     |> sync_state(state)
     |> schedule_effects(effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, _reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ),
      do: {:noreply, assign(socket, :snapshot_task, nil)}

  def handle_info({ref, event}, %{assigns: %{effect_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = AgentInstanceState.reduce(socket.assigns.page_state, event)
    effects = snapshot_after_successful_command(effects, event)

    {:noreply,
     socket
     |> assign(:effect_task, nil)
     |> sync_state(state)
     |> schedule_effects(effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{effect_task: %Task{ref: ref}}} = socket
      ) do
    {state, effects} =
      AgentInstanceState.reduce(socket.assigns.page_state, {:effect_failed, reason})

    {:noreply,
     socket
     |> assign(:effect_task, nil)
     |> sync_state(state)
     |> schedule_effects(effects)}
  end

  def handle_info(_message, socket) do
    {:noreply, socket}
  end

  @impl true
  def handle_event(event, params, socket) when event in @events do
    {state, effects} =
      AgentInstanceState.reduce(socket.assigns.page_state, {:interaction, event, params})

    {:noreply, socket |> sync_state(state) |> schedule_effects(effects)}
  end

  @impl true
  def terminate(_reason, socket) do
    cancel_effect(socket)
    PageStream.cancel(socket.assigns[:snapshot_task])
    :ok
  end

  @impl true
  def render(assigns) do
    ~H"""
    <Layouts.app
      flash={@flash}
      current_identity={@current_identity}
      organizations_destination="/organizations"
      logout_destination="/logout"
    >
      <AgentInstancePage.agent_instance
        state={@presentation.state}
        instance={@presentation.instance}
        revisions={@streams.revisions}
        attachments={@streams.attachments}
        updates={@streams.updates}
        attachment_form={to_form(@presentation.forms.attachment, as: :attachment)}
        revision_form={to_form(@presentation.forms.revision, as: :revision)}
        update_form={to_form(@presentation.forms.update, as: :update)}
        binding_form={to_form(@presentation.forms.binding, as: :binding)}
        capability_form={to_form(@presentation.forms.capabilities, as: :capabilities)}
        organization_index_destination={@presentation.destinations[:organization_index]}
        organization_destination={@presentation.destinations[:organization]}
        project_agents_destination={@presentation.destinations[:project_agents]}
        repositories_tab_destination={@presentation.destinations[:repositories_tab]}
        agents_tab_destination={@presentation.destinations[:agents_tab]}
        runs_tab_destination={@presentation.destinations[:runs_tab]}
        settings_tab_destination={@presentation.destinations[:settings_tab]}
        run_destination={fn run_id -> "/runs/#{run_id}" end}
        create_attachment_event="create-attachment"
        set_attachment_event="set-attachment"
        remove_attachment_event="remove-attachment"
        revise_instance_event="revise-instance"
        revise_capabilities_event="revise-capabilities"
        create_update_event="create-update"
        recover_update_event="recover-update"
        bind_secret_event="bind-secret"
      />
    </Layouts.app>
    """
  end

  defp schedule_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
      {:load, _generation, _instance_id} = effect, socket ->
        start_effect(socket, effect)

      {:command, _generation, _event, _payload} = effect, socket ->
        start_effect(socket, effect)

      :snapshot, socket ->
        PageStream.start_snapshot(socket, AgentInstanceState)

      {:flash, kind, message}, socket ->
        put_flash(socket, kind, message)

      {:navigate, destination}, socket ->
        push_navigate(socket, to: destination)
    end)
  end

  defp start_effect(socket, effect) do
    identity = socket.assigns.current_identity
    socket = cancel_effect(socket)

    task =
      Task.Supervisor.async_nolink(HephaestusWeb.PageTaskSupervisor, fn ->
        AgentInstanceState.execute(effect, identity)
      end)

    assign(socket, :effect_task, task)
  end

  defp cancel_effect(%{assigns: %{effect_task: %Task{} = task}} = socket) do
    Task.Supervisor.terminate_child(HephaestusWeb.PageTaskSupervisor, task.pid)
    assign(socket, :effect_task, nil)
  end

  defp cancel_effect(socket), do: socket

  # Commands return a committed receipt. Refresh the finite detail snapshot
  # directly instead of waiting for a product-event watch to observe it.
  defp snapshot_after_successful_command(
         effects,
         {:command_completed, _generation, {:ok, _receipt, _message}}
       ),
       do: effects ++ [:snapshot]

  defp snapshot_after_successful_command(effects, _event), do: effects

  defp sync_state(socket, state) do
    presentation = AgentInstanceState.present(state)

    socket
    |> assign(:page_state, state)
    |> assign(:cursor, state.cursor)
    |> assign(:stream_generation, state.stream_generation)
    |> assign(:presentation, presentation)
    |> assign(:page_title, page_title(presentation.instance))
    |> stream(:revisions, presentation.revisions, reset: true)
    |> stream(:attachments, presentation.attachments, reset: true)
    |> stream(:updates, presentation.updates, reset: true)
  end

  defp page_title(nil), do: "Agent"
  defp page_title(instance), do: "#{instance["name"]} · Agent"
end
