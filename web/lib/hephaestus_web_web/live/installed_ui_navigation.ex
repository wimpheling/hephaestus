defmodule HephaestusWebWeb.InstalledUiNavigation do
  @moduledoc false

  import Phoenix.Component, only: [assign: 3]
  import Phoenix.LiveView, only: [connected?: 1, put_flash: 3, push_event: 3]

  alias HephaestusWebWeb.InstalledUiNavigationEffects

  @refresh_interval_ms 30_000

  @doc "Initializes one navigation target and starts its connected-only load."
  def initialize(socket, organization_id, target) do
    state = InstalledUiNavigationEffects.new(%{organization_id: organization_id, target: target})

    socket
    |> assign(:installed_ui_state, state)
    |> assign(:installed_ui, InstalledUiNavigationEffects.present(state))
    |> assign(:installed_ui_task, nil)
    |> assign(:installed_ui_refresh_timer, nil)
    |> maybe_start(socket)
  end

  @doc "Completes a navigation load and refreshes its presentation assign."
  def complete(socket, event) do
    previous = socket.assigns.installed_ui_state
    reduced_event = normalize_task_event(event)
    {state, effects} = InstalledUiNavigationEffects.reduce(previous, reduced_event)

    socket
    |> maybe_clear_task(loaded_event?(reduced_event))
    |> assign(:installed_ui_state, state)
    |> assign(:installed_ui, InstalledUiNavigationEffects.present(state))
    |> maybe_close_active_frame(previous, state, loaded_event?(reduced_event))
    |> apply_effects(effects)
  end

  @doc "Starts a bounded refresh if no navigation task is in flight."
  def refresh(socket) do
    case {Map.get(socket.assigns, :installed_ui_state),
          Map.get(socket.assigns, :installed_ui_task)} do
      {nil, _task} ->
        socket

      {state, nil} ->
        dispatch(socket, :refresh, state)

      {_state, _task} ->
        socket
    end
  end

  @doc "Starts a bounded next-page request for the current organization and target."
  def load_more(socket) do
    case {Map.get(socket.assigns, :installed_ui_state),
          Map.get(socket.assigns, :installed_ui_task)} do
      {%{data: %{next_page_token: token}} = state, nil} when is_binary(token) ->
        dispatch(socket, :load_more, state)

      _unchanged ->
        socket
    end
  end

  @doc "Clears the active browser selection after an explicit close action."
  def close(socket) do
    dispatch(socket, :close)
  end

  @doc "Launches only the current safe entry and pushes a one-shot browser event."
  def launch(socket, installation_id) do
    case Map.get(socket.assigns, :installed_ui_state) do
      nil -> put_flash(socket, :error, "Installed UIs are still loading.")
      state -> dispatch(socket, {:launch, installation_id}, state)
    end
  end

  @doc "Clears navigation authority and requests that the browser hide the frame."
  def terminate(socket) do
    dispatch(socket, :terminate)
  end

  @doc "Schedules the next periodic authority refresh after a connected load."
  def schedule_refresh(socket) do
    socket = cancel_refresh_timer(socket)

    if connected?(socket) do
      assign(
        socket,
        :installed_ui_refresh_timer,
        Process.send_after(self(), :refresh_installed_ui, @refresh_interval_ms)
      )
    else
      socket
    end
  end

  @doc "Cancels the single pending authority refresh timer for a shell."
  def cancel_refresh_timer(socket) do
    case Map.get(socket.assigns, :installed_ui_refresh_timer) do
      timer when is_reference(timer) ->
        Process.cancel_timer(timer)
        assign(socket, :installed_ui_refresh_timer, nil)

      _missing ->
        socket
    end
  end

  defp maybe_close_active_frame(socket, previous, current, true) do
    if active_ref(previous) != active_ref(current) and active_ref(previous) != nil do
      reason =
        case current.status do
          :error -> "unavailable"
          :access_revoked -> "access_revoked"
          _status -> "active_generation_changed"
        end

      push_event(socket, "ui-browser-close", %{"reason" => reason})
    else
      socket
    end
  end

  defp maybe_close_active_frame(socket, _previous, _current, false), do: socket

  defp active_ref(%{
         data: %{active_installation_id: installation_id, active_generation_id: generation_id}
       })
       when is_binary(installation_id) and is_binary(generation_id),
       do: {installation_id, generation_id}

  defp active_ref(_state), do: nil

  defp maybe_start(socket, original_socket) do
    if connected?(original_socket), do: dispatch(socket, :load), else: socket
  end

  defp dispatch(socket, event, state \\ nil) do
    state = state || socket.assigns.installed_ui_state
    {state, effects} = InstalledUiNavigationEffects.reduce(state, event)

    socket
    |> assign(:installed_ui_state, state)
    |> assign(:installed_ui, InstalledUiNavigationEffects.present(state))
    |> apply_effects(effects)
  end

  defp apply_effects(socket, effects) do
    Enum.reduce(effects, socket, fn
      {:load, generation}, socket ->
        start_effect(socket, {:load, socket.assigns.current_identity, generation})

      {:handoff, entry}, socket ->
        state = socket.assigns.installed_ui_state
        options = Map.get(socket.private, :installed_ui_navigation_handoff_options, [])

        event =
          InstalledUiNavigationEffects.execute(
            state,
            {:handoff, socket.assigns.current_identity, entry, options}
          )

        {state, follow_up_effects} = InstalledUiNavigationEffects.reduce(state, event)

        socket
        |> assign(:installed_ui_state, state)
        |> assign(:installed_ui, InstalledUiNavigationEffects.present(state))
        |> apply_effects(follow_up_effects)

      {:close, reason}, socket ->
        push_event(socket, "ui-browser-close", %{"reason" => reason})

      {:launch, entry, url}, socket ->
        push_event(socket, "ui-browser-launch", %{
          "url" => url,
          "mode" => entry["presentation"],
          "installation_id" => entry["installation_id"]
        })

      {:flash, level, message}, socket ->
        put_flash(socket, level, message)

      _effect, socket ->
        socket
    end)
  end

  defp start_effect(socket, command) do
    state = socket.assigns.installed_ui_state
    task = Task.async(fn -> InstalledUiNavigationEffects.execute(state, command) end)
    assign(socket, :installed_ui_task, task)
  end

  defp maybe_clear_task(socket, true), do: assign(socket, :installed_ui_task, nil)
  defp maybe_clear_task(socket, false), do: socket

  defp normalize_task_event({:loaded, _result} = event), do: event
  defp normalize_task_event({:handoff_result, _entry, _result} = event), do: event
  defp normalize_task_event(event), do: {:loaded, event}

  defp loaded_event?({:loaded, _result}), do: true
  defp loaded_event?(_event), do: false
end
