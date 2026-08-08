defmodule HephaestusWebWeb.RepositoryBuildsLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryBuildsPage
  alias HephaestusWebWeb.{RepositoryBuildsState, RepositoryLiveSupport}

  # Build history is refreshed by an explicit request, not a durable page watch.
  @stream_mode :none

  @impl true
  def mount(%{"repository_id" => repository_id}, _session, socket) do
    state = RepositoryBuildsState.new(repository_id)
    {:ok, RepositoryLiveSupport.initialize(socket, state, RepositoryBuildsState, @stream_mode)}
  end

  @impl true
  def handle_params(params, uri, socket) do
    {socket, effects} =
      socket
      |> cancel_effect()
      |> then(&RepositoryLiveSupport.reduce(&1, RepositoryBuildsState, {:load, params, uri}))

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryBuildsState, effects)}
  end

  @impl true
  def handle_event("request-build", %{"build" => attributes}, socket) do
    {socket, effects} =
      RepositoryLiveSupport.reduce(socket, RepositoryBuildsState, {:request_build, attributes})

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryBuildsState, effects)}
  end

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{stream_generation: generation}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.reduce_watch(socket, RepositoryBuildsState, response)

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryBuildsState, effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{stream_generation: generation}} = socket
      ) do
    {socket, effects} = RepositoryLiveSupport.reduce_ended(socket, RepositoryBuildsState, result)
    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryBuildsState, effects)}
  end

  def handle_info({:page_watch, _generation, _response}, socket), do: {:noreply, socket}
  def handle_info({:page_watch_ended, _generation, _result}, socket), do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])

    {socket, effects} =
      RepositoryLiveSupport.complete_snapshot(socket, RepositoryBuildsState, event)

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryBuildsState, effects)}
  end

  def handle_info({ref, event}, %{assigns: %{effect_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {socket, effects} = RepositoryLiveSupport.complete(socket, RepositoryBuildsState, event)
    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryBuildsState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.complete_snapshot(
        socket,
        RepositoryBuildsState,
        {:effect_failed, reason}
      )

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryBuildsState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{effect_task: %Task{ref: ref}}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.complete(socket, RepositoryBuildsState, {:effect_failed, reason})

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryBuildsState, effects)}
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
      <RepositoryBuildsPage.repository_builds
        state={@presentation.state}
        model={@presentation}
        builds={@streams.builds}
        build_request_form={to_form(@presentation.build_request_form, as: :build)}
        request_event="request-build"
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
