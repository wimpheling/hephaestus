defmodule HephaestusWebWeb.RepositoryLiveSupport do
  @moduledoc false

  import Phoenix.Component, only: [assign: 3]

  import Phoenix.LiveView,
    only: [connected?: 1, put_flash: 3, push_navigate: 2, push_patch: 2, stream: 4]

  alias HephaestusWebWeb.PageStream

  def initialize(socket, state, state_module, stream_mode) do
    presentation = state_module.present(state)

    socket =
      socket
      |> stream(:commits, [], dom_id: &"commit-#{&1.id}")
      |> stream(:branches, [], dom_id: &"branch-#{tree_id(&1.ref)}")
      |> stream(:releases, [], dom_id: &"release-#{&1["id"]}")
      |> stream(:attached_instances, [], dom_id: &"attachment-#{&1["id"]}")
      |> assign(:page_state, state)
      |> assign(:presentation, presentation)
      |> assign(:effect_task, nil)
      |> assign(:watch_task, nil)
      |> assign(:snapshot_task, nil)
      |> assign(:cursor, state.cursor)
      |> assign(:stream_generation, state.stream_generation)
      |> assign(:stream_mode, stream_mode)

    if connected?(socket) do
      socket = PageStream.start_watch(socket, state_module)
      sync(socket, socket.assigns.page_state, state_module)
    else
      socket
    end
  end

  def reduce(socket, state_module, event) do
    {state, effects} = state_module.reduce(socket.assigns.page_state, event)
    {sync(socket, state, state_module), effects}
  end

  def complete(socket, state_module, event) do
    socket = assign(socket, :effect_task, nil)
    reduce(socket, state_module, event)
  end

  def complete_snapshot(socket, state_module, event) do
    socket = assign(socket, :snapshot_task, nil)
    reduce(socket, state_module, event)
  end

  def reduce_watch(socket, state_module, response) do
    {socket, effects} = PageStream.reduce_watch(socket, state_module, response)
    {sync(socket, socket.assigns.page_state, state_module), effects}
  end

  def reduce_ended(socket, state_module, result) do
    {socket, effects} = PageStream.reduce_ended(socket, state_module, result)
    {sync(socket, socket.assigns.page_state, state_module), effects}
  end

  def start_effect(socket, state_module, effect) do
    identity = socket.assigns.current_identity

    task =
      Task.Supervisor.async_nolink(HephaestusWeb.PageTaskSupervisor, fn ->
        state_module.execute(effect, identity)
      end)

    assign(socket, :effect_task, task)
  end

  def apply_effects(socket, state_module, effects) do
    Enum.reduce(effects, socket, fn
      {:load, _generation, _action, _repository_id, _params, _uri} = effect, socket ->
        start_effect(socket, state_module, effect)

      :snapshot, socket ->
        PageStream.start_snapshot(socket, state_module)

      :replace_watch, socket ->
        PageStream.start_watch(socket, state_module, false)

      {:patch, destination}, socket ->
        push_patch(socket, to: destination)

      {:navigate, destination}, socket ->
        push_navigate(socket, to: destination)

      {:flash, kind, message}, socket ->
        put_flash(socket, kind, message)
    end)
  end

  def sync(socket, state, state_module) do
    presentation = state_module.present(state)

    socket
    |> assign(:page_state, state)
    |> assign(:presentation, presentation)
    |> assign(:cursor, state.cursor)
    |> assign(:stream_generation, state.stream_generation)
    |> assign(:page_title, page_title(presentation))
    |> stream(:commits, presentation.commits, reset: true)
    |> stream(:branches, presentation.branches, reset: true)
    |> stream(:releases, presentation.releases, reset: true)
    |> stream(:attached_instances, presentation.attached_instances, reset: true)
  end

  def cancel_streams(socket) do
    PageStream.cancel(socket.assigns[:watch_task])
    PageStream.cancel(socket.assigns[:snapshot_task])
    :ok
  end

  defp page_title(%{repository: nil}), do: "Repository"

  defp page_title(presentation),
    do: "#{presentation.repository["name"]} · #{tab_name(presentation.active)}"

  defp tab_name(:files), do: "Files"
  defp tab_name(:commits), do: "Commits"
  defp tab_name(:branches), do: "Branches"
  defp tab_name(:releases), do: "Releases"
  defp tab_name(:agents), do: "Agents"

  defp tree_id(value) do
    :crypto.hash(:sha256, value)
    |> Base.url_encode64(padding: false)
    |> binary_part(0, 12)
  end
end
