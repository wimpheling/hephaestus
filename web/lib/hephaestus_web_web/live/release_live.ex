defmodule HephaestusWebWeb.ReleaseLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.ReleasePage
  alias HephaestusWebWeb.{PageStream, ReleaseState}

  @stream_mode :page_scoped

  @impl true
  def mount(%{"release_id" => release_id}, _session, socket) do
    {state, effects} = release_id |> ReleaseState.new() |> ReleaseState.reduce(:load)

    socket =
      socket
      |> stream_configure(:artifacts, dom_id: &"release-artifact-#{&1["id"]}")
      |> stream_configure(:agents, dom_id: &"release-agent-#{&1["id"]}")
      |> stream(:artifacts, [])
      |> stream(:agents, [])
      |> assign(:page_state, state)
      |> assign(:presentation, ReleaseState.present(state))
      |> assign(:watch_task, nil)
      |> assign(:snapshot_task, nil)
      |> assign(:stream_mode, @stream_mode)

    {:ok, schedule_effects(socket, effects)}
  end

  @impl true
  def handle_async({:release_effect, _generation}, {:ok, event}, socket) do
    {state, effects} = ReleaseState.reduce(socket.assigns.page_state, event)

    socket = socket |> sync_state(state) |> schedule_effects(effects) |> maybe_start_watch()
    {:noreply, socket}
  end

  def handle_async({:release_effect, _generation}, {:exit, reason}, socket) do
    {state, effects} = ReleaseState.reduce(socket.assigns.page_state, {:effect_failed, reason})
    {:noreply, socket |> sync_state(state) |> schedule_effects(effects)}
  end

  @impl true
  def handle_event("set-draft-version", %{"release" => %{"version" => version}}, socket) do
    {state, effects} =
      ReleaseState.reduce(socket.assigns.page_state, {:set_draft_version, version})

    {:noreply, socket |> sync_state(state) |> schedule_effects(effects)}
  end

  def handle_event("publish-release", _params, socket) do
    {state, effects} = ReleaseState.reduce(socket.assigns.page_state, :publish_release)
    {:noreply, socket |> sync_state(state) |> schedule_effects(effects)}
  end

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_watch(socket, ReleaseState, response)
    {:noreply, socket |> sync_state(socket.assigns.page_state) |> schedule_effects(effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{page_state: %{stream_generation: generation}}} = socket
      ) do
    {socket, effects} = PageStream.reduce_ended(socket, ReleaseState, result)
    {:noreply, socket |> sync_state(socket.assigns.page_state) |> schedule_effects(effects)}
  end

  def handle_info({:page_watch, _generation, _response}, socket), do: {:noreply, socket}
  def handle_info({:page_watch_ended, _generation, _result}, socket), do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {state, effects} = ReleaseState.reduce(socket.assigns.page_state, event)

    socket =
      socket
      |> assign(:snapshot_task, nil)
      |> sync_state(state)
      |> schedule_effects(effects)

    {:noreply, socket}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    {state, effects} = ReleaseState.reduce(socket.assigns.page_state, {:effect_failed, reason})

    {:noreply,
     socket
     |> assign(:snapshot_task, nil)
     |> sync_state(state)
     |> schedule_effects(effects)}
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
      <ReleasePage.release
        state={@presentation.state}
        release={@presentation.release}
        artifacts={@streams.artifacts}
        agents={@streams.agents}
        organization_index_destination={@presentation.destinations[:organization_index]}
        organization_destination={@presentation.destinations[:organization]}
        project_destination={@presentation.destinations[:project]}
        repository_releases_destination={@presentation.destinations[:repository_releases]}
        source_destination={@presentation.destinations[:source]}
        draft_version_form={to_form(@presentation.draft_version, as: :release)}
        set_draft_version_event={@presentation.set_draft_version_event}
        publish_event={@presentation.publish_event}
      />
    </Layouts.app>
    """
  end

  defp schedule_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
      {:load, generation, _release_id} = effect, socket ->
        identity = socket.assigns.current_identity

        start_async(socket, {:release_effect, generation}, fn ->
          ReleaseState.execute(effect, identity)
        end)

      {:set_draft_version, generation, _release_id, _version} = effect, socket ->
        identity = socket.assigns.current_identity

        start_async(socket, {:release_effect, generation}, fn ->
          ReleaseState.execute(effect, identity)
        end)

      {:publish_release, generation, _release_id} = effect, socket ->
        identity = socket.assigns.current_identity

        start_async(socket, {:release_effect, generation}, fn ->
          ReleaseState.execute(effect, identity)
        end)

      {:flash, kind, message}, socket ->
        put_flash(socket, kind, message)

      :snapshot, socket ->
        PageStream.start_snapshot(socket, ReleaseState)

      :replace_watch, socket ->
        PageStream.start_watch(socket, ReleaseState, false)

      {:navigate, :organizations}, socket ->
        socket
        |> put_flash(:error, socket.assigns.page_state.error)
        |> push_navigate(to: "/organizations")

      {:navigate, destination}, socket ->
        socket
        |> put_flash(:error, socket.assigns.page_state.error)
        |> push_navigate(to: destination)
    end)
  end

  defp sync_state(socket, state) do
    presentation = ReleaseState.present(state)

    socket
    |> assign(:page_state, state)
    |> assign(:presentation, presentation)
    |> assign(:page_title, page_title(presentation.release))
    |> stream(:artifacts, presentation.artifacts, reset: true)
    |> stream(:agents, presentation.agents, reset: true)
  end

  defp maybe_start_watch(
         %{assigns: %{watch_task: nil, page_state: %{status: :ready, data: %{release: release}}}} =
           socket
       )
       when not is_nil(release) do
    PageStream.start_watch(socket, ReleaseState)
  end

  defp maybe_start_watch(socket), do: socket

  defp page_title(nil), do: "Release"
  defp page_title(release), do: "#{release["version"]} · Release"
end
