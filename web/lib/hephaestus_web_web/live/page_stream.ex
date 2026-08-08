defmodule HephaestusWebWeb.PageStream do
  @moduledoc false

  import Phoenix.Component, only: [assign: 3]
  import Phoenix.LiveView, only: [put_flash: 3, push_navigate: 2]

  def start_watch(socket, state_module, increment? \\ true) do
    cancel(socket.assigns[:watch_task])
    state = socket.assigns.page_state
    state = if increment?, do: state_module.begin_watch(state), else: state
    identity = socket.assigns.current_identity
    generation = state.stream_generation
    # The generic stream owner deliberately reads the committed cursor before
    # delegating so reconnects cannot accidentally resume from rendered state.
    _committed_cursor = state.cursor
    owner = self()

    {:ok, task} =
      Task.Supervisor.start_child(HephaestusWeb.PageTaskSupervisor, fn ->
        result = state_module.watch(identity, state, owner, generation)
        send(owner, {:page_watch_ended, generation, result})
      end)

    socket |> assign(:page_state, state) |> assign(:watch_task, task)
  end

  def start_snapshot(socket, state_module) do
    cancel(socket.assigns[:snapshot_task])
    state = socket.assigns.page_state
    identity = socket.assigns.current_identity
    generation = state.stream_generation

    # Snapshot RPCs are finite page work. Do not queue them behind the
    # supervisor's long-lived product-event watch tasks.
    task = Task.async(fn -> state_module.execute(state, {:load, identity, generation}) end)

    assign(socket, :snapshot_task, task)
  end

  def reduce_watch(socket, state_module, response) do
    {state, effects} = state_module.reduce(socket.assigns.page_state, {:watch, response})
    {assign(socket, :page_state, state), effects}
  end

  def reduce_ended(socket, state_module, result \\ :ok) do
    {state, effects} =
      case result do
        {:error, %HephaestusWeb.RPC.Error{kind: kind}}
        when kind in [:not_found, :permission_denied, :unauthenticated] ->
          if function_exported?(state_module, :watch_ended, 2) do
            state_module.watch_ended(socket.assigns.page_state, result)
          else
            HephaestusWebWeb.ProductEventReducer.watch_ended(socket.assigns.page_state, result)
          end

        _other ->
          state_module.reduce(socket.assigns.page_state, :watch_ended)
      end

    {socket |> assign(:page_state, state) |> assign(:watch_task, nil), effects}
  end

  def apply_effects(socket, state_module, effects) do
    Enum.reduce(effects, socket, fn
      :snapshot, socket -> start_snapshot(socket, state_module)
      :replace_watch, socket -> start_watch(socket, state_module, false)
      {:flash, kind, message}, socket -> put_flash(socket, kind, message)
      {:navigate, :organizations}, socket -> push_navigate(socket, to: "/organizations")
    end)
  end

  def cancel(nil), do: :ok
  def cancel(%Task{pid: pid}), do: cancel(pid)

  def cancel(pid) when is_pid(pid),
    do: Task.Supervisor.terminate_child(HephaestusWeb.PageTaskSupervisor, pid)
end
