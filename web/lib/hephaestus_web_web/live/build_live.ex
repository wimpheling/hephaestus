defmodule HephaestusWebWeb.BuildLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.BuildPage
  alias HephaestusWebWeb.{BuildState, PageStream}

  @stream_mode :page_scoped

  @impl true
  def mount(%{"repository_id" => repository_id, "build_id" => build_id}, _session, socket) do
    _stream_mode = @stream_mode
    state = BuildState.new(repository_id, build_id)

    socket =
      socket
      |> assign(:page_state, state)
      |> assign(:presentation, BuildState.present(state))
      |> assign(:watch_task, nil)
      |> assign(:snapshot_task, nil)

    {:ok, socket}
  end

  @impl true
  def handle_params(_params, _uri, socket) do
    {state, effects} = BuildState.reduce(socket.assigns.page_state, :load)

    {:noreply, socket |> sync_state(state) |> schedule_effects(effects)}
  end

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {state, effects} = BuildState.reduce(socket.assigns.page_state, {:watch, response})
    {:noreply, socket |> sync_state(state) |> schedule_effects(effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {state, effects} = PageStream.reduce_ended(socket, BuildState, result)
    {:noreply, socket |> sync_state(state.assigns.page_state) |> schedule_effects(effects)}
  end

  def handle_info({:page_watch, _generation, _response}, socket), do: {:noreply, socket}
  def handle_info({:page_watch_ended, _generation, _result}, socket), do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = BuildState.reduce(socket.assigns.page_state, event)

    {:noreply,
     socket
     |> assign(:snapshot_task, nil)
     |> sync_state(state)
     |> schedule_effects(effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    {state, effects} = BuildState.reduce(socket.assigns.page_state, {:effect_failed, reason})

    {:noreply,
     socket |> assign(:snapshot_task, nil) |> sync_state(state) |> schedule_effects(effects)}
  end

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket) do
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
      <BuildPage.build
        state={@presentation.state}
        build={@presentation.build}
        repository={@presentation.repository}
        logs={@presentation.logs}
        metrics={@presentation.metrics}
        timeline={@presentation.timeline}
        declared_artifacts={@presentation.declared_artifacts}
        produced_artifacts={@presentation.produced_artifacts}
        artifact_manifest={@presentation.artifact_manifest}
        organization_index_destination={@presentation.destinations[:organization_index]}
        organization_destination={@presentation.destinations[:organization]}
        project_destination={@presentation.destinations[:project]}
        repository_destination={@presentation.destinations[:repository]}
        release_destination={@presentation.destinations[:release]}
      />
    </Layouts.app>
    """
  end

  defp schedule_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
      {:load, _generation, _repository_id, _build_id}, socket ->
        PageStream.start_snapshot(socket, BuildState)

      :snapshot, socket ->
        PageStream.start_snapshot(socket, BuildState)

      :replace_watch, socket ->
        PageStream.start_watch(socket, BuildState, false)

      {:flash, kind, message}, socket ->
        put_flash(socket, kind, message)

      {:navigate, destination}, socket ->
        push_navigate(socket, to: destination)
    end)
  end

  defp sync_state(socket, state) do
    socket
    |> assign(:page_state, state)
    |> assign(:presentation, BuildState.present(state))
    |> assign(:page_title, page_title(state))
  end

  defp page_title(%{data: %{build: build}}) when is_map(build), do: "#{build["id"]} · Build"
  defp page_title(_state), do: "Build"
end
