defmodule HephaestusWebWeb.RepositoryAgentsLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryAgentsPage
  alias HephaestusWebWeb.{RepositoryAgentsState, RepositoryLiveSupport}

  @stream_mode :page_scoped

  @impl true
  def mount(%{"repository_id" => repository_id}, _session, socket) do
    state = RepositoryAgentsState.new(repository_id)
    {:ok, RepositoryLiveSupport.initialize(socket, state, RepositoryAgentsState, @stream_mode)}
  end

  @impl true
  def handle_params(params, uri, socket) do
    {socket, effects} =
      socket
      |> cancel_effect()
      |> then(&RepositoryLiveSupport.reduce(&1, RepositoryAgentsState, {:load, params, uri}))

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryAgentsState, effects)}
  end

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{stream_generation: generation}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.reduce_watch(socket, RepositoryAgentsState, response)

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryAgentsState, effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{stream_generation: generation}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.reduce_ended(socket, RepositoryAgentsState, result)

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryAgentsState, effects)}
  end

  def handle_info({:page_watch, _generation, _response}, socket), do: {:noreply, socket}
  def handle_info({:page_watch_ended, _generation, _result}, socket), do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])

    {socket, effects} =
      RepositoryLiveSupport.complete_snapshot(socket, RepositoryAgentsState, event)

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryAgentsState, effects)}
  end

  def handle_info({ref, event}, %{assigns: %{effect_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {socket, effects} = RepositoryLiveSupport.complete(socket, RepositoryAgentsState, event)
    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryAgentsState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.complete_snapshot(
        socket,
        RepositoryAgentsState,
        {:effect_failed, reason}
      )

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryAgentsState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{effect_task: %Task{ref: ref}}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.complete(socket, RepositoryAgentsState, {:effect_failed, reason})

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryAgentsState, effects)}
  end

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket) do
    cancel_effect(socket)
    RepositoryLiveSupport.cancel_streams(socket)
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
      <RepositoryAgentsPage.repository_agents
        state={@presentation.state}
        model={@presentation}
        attachments={@streams.attached_instances}
      />
    </Layouts.app>
    """
  end

  defp cancel_effect(%{assigns: %{effect_task: %Task{} = task}} = socket) do
    Task.shutdown(task, :brutal_kill)
    assign(socket, :effect_task, nil)
  end

  defp cancel_effect(socket), do: socket
end
