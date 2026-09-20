defmodule HephaestusWebWeb.InstalledUiNavigationLive do
  @moduledoc false

  import Phoenix.Component, only: [assign: 3, update: 3]
  import Phoenix.LiveView, only: [connected?: 1, put_flash: 3, push_event: 3]

  alias HephaestusWeb.RPC.Client
  alias HephaestusWeb.UIBrowser
  alias HephaestusWebWeb.InstalledUiNavigationState

  @refresh_interval_ms 30_000

  @doc "Initializes one navigation target and starts its connected-only load."
  def initialize(socket, organization_id, target) do
    state = InstalledUiNavigationState.new(organization_id, target)

    socket
    |> assign(:installed_ui_state, state)
    |> assign(:installed_ui, InstalledUiNavigationState.present(state))
    |> assign(:installed_ui_task, nil)
    |> assign(:installed_ui_refresh_timer, nil)
    |> maybe_start(socket)
  end

  @doc "Completes a navigation load and refreshes its presentation assign."
  def complete(socket, event) do
    previous = socket.assigns.installed_ui_state
    state = InstalledUiNavigationState.complete(previous, event)

    socket
    |> assign(:installed_ui_task, nil)
    |> assign(:installed_ui_state, state)
    |> assign(:installed_ui, InstalledUiNavigationState.present(state))
    |> maybe_close_active_frame(previous, state)
  end

  @doc "Starts a bounded refresh if no navigation task is in flight."
  def refresh(socket) do
    case {Map.get(socket.assigns, :installed_ui_state),
          Map.get(socket.assigns, :installed_ui_task)} do
      {nil, _task} ->
        socket

      {state, nil} ->
        state = InstalledUiNavigationState.refresh(state)

        socket
        |> assign(:installed_ui_state, state)
        |> assign(:installed_ui, InstalledUiNavigationState.present(state))
        |> refresh_loaded()

      {_state, _task} ->
        socket
    end
  end

  @doc "Starts a bounded next-page request for the current organization and target."
  def load_more(socket) do
    case {Map.get(socket.assigns, :installed_ui_state),
          Map.get(socket.assigns, :installed_ui_task)} do
      {%{next_page_token: token} = state, nil} when is_binary(token) ->
        state = InstalledUiNavigationState.load_more(state)

        socket
        |> assign(:installed_ui_state, state)
        |> assign(:installed_ui, InstalledUiNavigationState.present(state))
        |> start()

      _unchanged ->
        socket
    end
  end

  @doc "Clears the active browser selection after an explicit close action."
  def close(socket) do
    state = InstalledUiNavigationState.deactivate(socket.assigns.installed_ui_state)

    socket
    |> assign(:installed_ui_state, state)
    |> assign(:installed_ui, InstalledUiNavigationState.present(state))
    |> push_event("ui-browser-close", %{"reason" => "user_closed"})
  end

  defp refresh_loaded(socket) do
    case socket.assigns.installed_ui_task do
      nil -> start(socket)
      _task -> socket
    end
  end

  @doc "Launches only the current safe entry and pushes a one-shot browser event."
  def launch(socket, installation_id) do
    state = Map.get(socket.assigns, :installed_ui_state)

    case state && InstalledUiNavigationState.launch_entry(state, installation_id) do
      {:ok, entry} -> launch_entry(socket, entry)
      {:error, :access_revoked} -> revoke(socket)
      {:error, :unavailable} -> put_flash(socket, :error, "That installed UI is unavailable.")
      _missing -> put_flash(socket, :error, "Installed UIs are still loading.")
    end
  end

  @doc "Clears navigation authority and requests that the browser hide the frame."
  def terminate(socket) do
    state = InstalledUiNavigationState.terminate(socket.assigns.installed_ui_state)

    socket
    |> assign(:installed_ui_state, state)
    |> assign(:installed_ui, InstalledUiNavigationState.present(state))
    |> push_event("ui-browser-close", %{"reason" => "terminated"})
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

  defp launch_entry(socket, entry) do
    identity = socket.assigns.current_identity

    result =
      Client.create_ui_browser_handoff(
        identity,
        entry["installation_id"],
        entry["generation_id"],
        entry["route_base"],
        on_success: fn handoff, secret ->
          projection = Map.put(handoff, "route_base", entry["route_base"])
          UIBrowser.bootstrap_url(projection, secret, "light")
        end
      )

    case result do
      {:ok, url} ->
        socket
        |> update(:installed_ui_state, &InstalledUiNavigationState.activate(&1, entry))
        |> push_event("ui-browser-launch", %{
          "url" => url,
          "mode" => entry["presentation"],
          "installation_id" => entry["installation_id"]
        })

      {:error, %HephaestusWeb.RPC.Error{kind: kind}}
      when kind in [:permission_denied, :not_found] ->
        revoke(socket)

      {:error, _reason} ->
        put_flash(socket, :error, "The installed UI could not be launched.")
    end
  end

  defp revoke(socket) do
    state = InstalledUiNavigationState.terminate(socket.assigns.installed_ui_state)

    socket
    |> assign(:installed_ui_state, state)
    |> assign(:installed_ui, InstalledUiNavigationState.present(state))
    |> push_event("ui-browser-close", %{"reason" => "access_revoked"})
    |> put_flash(:error, "Installed UI access was revoked.")
  end

  defp maybe_close_active_frame(socket, previous, current) do
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

  defp active_ref(%{active_installation_id: installation_id, active_generation_id: generation_id})
       when is_binary(installation_id) and is_binary(generation_id),
       do: {installation_id, generation_id}

  defp active_ref(_state), do: nil

  defp maybe_start(socket, original_socket) do
    if connected?(original_socket), do: start(socket), else: socket
  end

  defp start(socket) do
    identity = socket.assigns.current_identity
    state = socket.assigns.installed_ui_state
    task = Task.async(fn -> InstalledUiNavigationState.execute(state, identity) end)
    assign(socket, :installed_ui_task, task)
  end
end
