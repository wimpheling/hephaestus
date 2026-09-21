defmodule HephaestusWebWeb.RepositoryFilesLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryFilesPage

  alias HephaestusWebWeb.{
    InstalledUiNavigation,
    RepositoryFilesState,
    RepositoryLiveSupport
  }

  @stream_mode :none

  @impl true
  def mount(%{"repository_id" => repository_id}, _session, socket) do
    state = RepositoryFilesState.new(repository_id)

    socket = RepositoryLiveSupport.initialize(socket, state, RepositoryFilesState, @stream_mode)

    {:ok,
     socket
     |> assign(:installed_ui_state, nil)
     |> assign(:installed_ui, %{
       state: :loading,
       installations: [],
       error: nil,
       has_more: false,
       loading_more: false
     })
     |> assign(:installed_ui_task, nil)}
  end

  @impl true
  def handle_params(params, uri, socket) do
    {socket, effects} =
      socket
      |> cancel_effect()
      |> then(&RepositoryLiveSupport.reduce(&1, RepositoryFilesState, {:load, params, uri}))

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryFilesState, effects)}
  end

  @impl true
  def handle_event("select-branch", %{"browse" => %{"branch" => branch}}, socket) do
    {socket, effects} =
      RepositoryLiveSupport.reduce(socket, RepositoryFilesState, {:select_branch, branch})

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryFilesState, effects)}
  end

  @impl true
  def handle_event("launch-installed-ui", %{"id" => installation_id}, socket),
    do: {:noreply, InstalledUiNavigation.launch(socket, installation_id)}

  def handle_event("refresh-installed-ui", _params, socket),
    do: {:noreply, InstalledUiNavigation.refresh(socket)}

  def handle_event("load-more-installed-ui", _params, socket),
    do: {:noreply, InstalledUiNavigation.load_more(socket)}

  def handle_event("close-installed-ui", _params, socket),
    do: {:noreply, InstalledUiNavigation.close(socket)}

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{stream_generation: generation}} = socket
      ) do
    {socket, effects} = RepositoryLiveSupport.reduce_watch(socket, RepositoryFilesState, response)

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryFilesState, effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{stream_generation: generation}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.reduce_ended(socket, RepositoryFilesState, result)

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryFilesState, effects)}
  end

  def handle_info({:page_watch, _generation, _response}, socket), do: {:noreply, socket}
  def handle_info({:page_watch_ended, _generation, _result}, socket), do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])

    {socket, effects} =
      RepositoryLiveSupport.complete_snapshot(socket, RepositoryFilesState, event)

    socket = maybe_start_installed_ui(socket)

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryFilesState, effects)}
  end

  def handle_info(
        {ref, event},
        %{assigns: %{installed_ui_task: %Task{ref: ref}}} = socket
      ) do
    Process.demonitor(ref, [:flush])

    {:noreply,
     socket
     |> InstalledUiNavigation.complete(event)
     |> InstalledUiNavigation.schedule_refresh()}
  end

  def handle_info(:refresh_installed_ui, socket),
    do:
      {:noreply,
       socket
       |> InstalledUiNavigation.refresh()
       |> InstalledUiNavigation.schedule_refresh()}

  def handle_info({ref, event}, %{assigns: %{effect_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {socket, effects} = RepositoryLiveSupport.complete(socket, RepositoryFilesState, event)
    socket = maybe_start_installed_ui(socket)
    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryFilesState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.complete_snapshot(
        socket,
        RepositoryFilesState,
        {:effect_failed, reason}
      )

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryFilesState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{effect_task: %Task{ref: ref}}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.complete(socket, RepositoryFilesState, {:effect_failed, reason})

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryFilesState, effects)}
  end

  def handle_info(_message, socket), do: {:noreply, socket}

  @impl true
  def terminate(_reason, socket) do
    cancel_effect(socket)
    RepositoryLiveSupport.cancel_streams(socket)
    InstalledUiNavigation.cancel_refresh_timer(socket)
    cancel_installed_ui(socket.assigns[:installed_ui_task])
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
      <RepositoryFilesPage.repository_files
        state={@presentation.state}
        model={@presentation}
        branch_form={to_form(@presentation.browse_form, as: :browse)}
        select_branch_event="select-branch"
        installed_ui={@installed_ui}
      />
    </Layouts.app>
    """
  end

  defp cancel_effect(%{assigns: %{effect_task: %Task{} = task}} = socket) do
    Task.shutdown(task, :brutal_kill)
    assign(socket, :effect_task, nil)
  end

  defp cancel_effect(socket), do: socket

  defp maybe_start_installed_ui(%{assigns: %{installed_ui_state: nil}} = socket) do
    case socket.assigns.presentation.repository do
      %{"organization_id" => organization_id, "id" => repository_id} ->
        InstalledUiNavigation.initialize(
          socket,
          organization_id,
          {:repository, repository_id}
        )

      _repository ->
        socket
    end
  end

  defp maybe_start_installed_ui(socket), do: socket

  defp cancel_installed_ui(nil), do: :ok
  defp cancel_installed_ui(%Task{pid: pid}), do: Task.shutdown(pid, :brutal_kill)
end
