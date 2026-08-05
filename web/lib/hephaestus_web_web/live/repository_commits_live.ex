defmodule HephaestusWebWeb.RepositoryCommitsLive do
  use HephaestusWebWeb, :live_view

  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryCommitsPage
  alias HephaestusWebWeb.{RepositoryCommitsState, RepositoryLiveSupport}

  @stream_mode :none

  @impl true
  def mount(%{"repository_id" => repository_id}, _session, socket) do
    state = RepositoryCommitsState.new(repository_id)
    {:ok, RepositoryLiveSupport.initialize(socket, state, RepositoryCommitsState, @stream_mode)}
  end

  @impl true
  def handle_params(params, uri, socket) do
    {socket, effects} =
      socket
      |> cancel_effect()
      |> then(&RepositoryLiveSupport.reduce(&1, RepositoryCommitsState, {:load, params, uri}))

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryCommitsState, effects)}
  end

  @impl true
  def handle_event("select-branch", %{"browse" => %{"branch" => branch}}, socket) do
    {socket, effects} =
      RepositoryLiveSupport.reduce(socket, RepositoryCommitsState, {:select_branch, branch})

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryCommitsState, effects)}
  end

  @impl true
  def handle_info(
        {:page_watch, generation, response},
        %{assigns: %{stream_generation: generation}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.reduce_watch(socket, RepositoryCommitsState, response)

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryCommitsState, effects)}
  end

  def handle_info(
        {:page_watch_ended, generation, result},
        %{assigns: %{stream_generation: generation}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.reduce_ended(socket, RepositoryCommitsState, result)

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryCommitsState, effects)}
  end

  def handle_info({:page_watch, _generation, _response}, socket), do: {:noreply, socket}
  def handle_info({:page_watch_ended, _generation, _result}, socket), do: {:noreply, socket}

  def handle_info({ref, event}, %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])

    {socket, effects} =
      RepositoryLiveSupport.complete_snapshot(socket, RepositoryCommitsState, event)

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryCommitsState, effects)}
  end

  def handle_info({ref, event}, %{assigns: %{effect_task: %Task{ref: ref}}} = socket) do
    Process.demonitor(ref, [:flush])
    {socket, effects} = RepositoryLiveSupport.complete(socket, RepositoryCommitsState, event)
    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryCommitsState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{snapshot_task: %Task{ref: ref}}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.complete_snapshot(
        socket,
        RepositoryCommitsState,
        {:effect_failed, reason}
      )

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryCommitsState, effects)}
  end

  def handle_info(
        {:DOWN, ref, :process, _pid, reason},
        %{assigns: %{effect_task: %Task{ref: ref}}} = socket
      ) do
    {socket, effects} =
      RepositoryLiveSupport.complete(socket, RepositoryCommitsState, {:effect_failed, reason})

    {:noreply, RepositoryLiveSupport.apply_effects(socket, RepositoryCommitsState, effects)}
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
      <RepositoryCommitsPage.repository_commits
        state={@presentation.state}
        model={@presentation}
        commits={@streams.commits}
        branch_form={to_form(@presentation.browse_form, as: :browse)}
        select_branch_event="select-branch"
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
