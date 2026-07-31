defmodule HephaestusWebWeb.RunLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.RunPage
  alias HephaestusWebWeb.PageStream
  alias HephaestusWebWeb.RunState

  @stream_mode :page_scoped

  @impl true
  def mount(%{"run_id" => run_id}, _session, socket) do
    state = RunState.new(run_id)

    socket =
      socket
      |> stream_configure(:events, dom_id: &"event-#{&1["sequence"]}")
      |> stream_configure(:artifacts, dom_id: &"artifact-#{&1["id"]}")
      |> stream(:events, [])
      |> stream(:artifacts, [])
      |> assign(:page_state, state)
      |> assign(:presentation, RunState.present(state))
      |> assign(:effect_task, nil)
      |> assign(:watch_task, nil)
      |> assign(:snapshot_task, nil)
      |> assign(:cursor, state.cursor)
      |> assign(:stream_generation, state.stream_generation)
      |> assign(:stream_mode, @stream_mode)

    if connected?(socket),
      do: {:ok, PageStream.start_watch(socket, RunState)},
      else: {:ok, socket}
  end

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_watch(socket, RunState, response)
    {:noreply, socket |> sync_state(socket.assigns.page_state) |> schedule_effects(effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_ended(socket, RunState, result)
    {:noreply, socket |> sync_state(socket.assigns.page_state) |> schedule_effects(effects)}
  end

  def handle_info({kind, _generation, _value}, socket)
      when kind in [:page_watch, :page_watch_ended],
      do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = RunState.reduce(socket.assigns.page_state, event)

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
    {state, effects} = RunState.reduce(socket.assigns.page_state, event)

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
    {state, effects} = RunState.reduce(socket.assigns.page_state, {:effect_failed, reason})

    {:noreply,
     socket
     |> assign(:effect_task, nil)
     |> sync_state(state)
     |> schedule_effects(effects)}
  end

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def handle_event("control", params, socket) do
    {state, effects} = RunState.reduce(socket.assigns.page_state, {:control, params})
    {:noreply, socket |> sync_state(state) |> schedule_effects(effects)}
  end

  @impl true
  def terminate(_reason, socket) do
    cancel_effect(socket)
    PageStream.cancel(socket.assigns[:watch_task])
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
      <RunPage.run
        state={@presentation.state}
        run={@presentation.run}
        patch={@presentation.patch}
        manifest={@presentation.manifest}
        events={@streams.events}
        artifacts={@streams.artifacts}
        organization_index_destination={@presentation.destinations[:organization_index]}
        organization_destination={@presentation.destinations[:organization]}
        repository_destination={@presentation.destinations[:repository]}
        release_destination={@presentation.destinations[:release]}
        agent_destination={@presentation.destinations[:agent]}
        control_event="control"
      />
    </Layouts.app>
    """
  end

  defp schedule_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
      {kind, _generation, _payload} = effect, socket when kind in [:load, :control] ->
        identity = socket.assigns.current_identity
        socket = cancel_effect(socket)

        task =
          Task.Supervisor.async_nolink(HephaestusWeb.PageTaskSupervisor, fn ->
            RunState.execute(effect, identity)
          end)

        assign(socket, :effect_task, task)

      :snapshot, socket ->
        PageStream.start_snapshot(socket, RunState)

      :replace_watch, socket ->
        PageStream.start_watch(socket, RunState, false)

      {:flash, kind, message}, socket ->
        put_flash(socket, kind, message)

      {:navigate, destination}, socket ->
        push_navigate(socket, to: destination)
    end)
  end

  defp sync_state(socket, state) do
    presentation = RunState.present(state)

    socket
    |> assign(:page_state, state)
    |> assign(:cursor, state.cursor)
    |> assign(:stream_generation, state.stream_generation)
    |> assign(:presentation, presentation)
    |> assign(:page_title, page_title(presentation.run))
    |> stream(:events, presentation.events, reset: true)
    |> stream(:artifacts, presentation.artifacts, reset: true)
  end

  defp cancel_effect(%{assigns: %{effect_task: %Task{} = task}} = socket) do
    Task.Supervisor.terminate_child(HephaestusWeb.PageTaskSupervisor, task.pid)
    assign(socket, :effect_task, nil)
  end

  defp cancel_effect(socket), do: socket

  defp page_title(nil), do: "Run"
  defp page_title(run), do: "Run #{String.slice(run["id"], 0, 8)}"
end
