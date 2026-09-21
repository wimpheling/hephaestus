defmodule HephaestusWebWeb.InstalledUiNavigationEffectsTest do
  use ExUnit.Case, async: true

  alias Hephaestus.Common.V1.OpaqueId
  alias Hephaestus.Release.V1.CreateUiBrowserHandoffResponse
  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.Error
  alias HephaestusWebWeb.{InstalledUiNavigation, InstalledUiNavigationEffects}

  @installation %{
    "installation_id" => "installation-1",
    "generation_id" => "generation-1",
    "lifecycle" => "enabled",
    "launchable" => true,
    "route_base" => "docs"
  }

  test "closes the active frame as access revoked after a denied refresh" do
    socket = socket_for(InstalledUiNavigationEffects.load_more(active_state()))

    updated =
      InstalledUiNavigation.complete(
        socket,
        {:error, %Error{kind: :permission_denied, retryable: false}}
      )

    assert updated.assigns.installed_ui_state.status == :access_revoked

    assert [["ui-browser-close", %{"reason" => "access_revoked"}]] ==
             Phoenix.LiveView.Utils.get_push_events(updated)
  end

  test "closes the active frame as unavailable after an error refresh" do
    socket = socket_for(InstalledUiNavigationEffects.load_more(active_state()))

    updated =
      InstalledUiNavigation.complete(
        socket,
        {:error, %Error{kind: :invalid, retryable: false}}
      )

    assert updated.assigns.installed_ui_state.status == :error

    assert [["ui-browser-close", %{"reason" => "unavailable"}]] ==
             Phoenix.LiveView.Utils.get_push_events(updated)
  end

  test "closes the active frame when its generation is replaced" do
    replacement = %{@installation | "generation_id" => "generation-2"}

    updated =
      InstalledUiNavigation.complete(socket_for(active_state()), response(replacement))

    assert updated.assigns.installed_ui_state.data.active_generation_id == nil

    assert [["ui-browser-close", %{"reason" => "active_generation_changed"}]] ==
             Phoenix.LiveView.Utils.get_push_events(updated)
  end

  test "closes the active frame when its installation is disabled" do
    disabled = %{@installation | "lifecycle" => "disabled", "launchable" => false}
    updated = InstalledUiNavigation.complete(socket_for(active_state()), response(disabled))

    assert updated.assigns.installed_ui_state.data.active_installation_id == nil

    assert [["ui-browser-close", %{"reason" => "active_generation_changed"}]] ==
             Phoenix.LiveView.Utils.get_push_events(updated)
  end

  test "closes the active frame when its installation is removed" do
    removed = %{@installation | "lifecycle" => "removed", "launchable" => false}
    updated = InstalledUiNavigation.complete(socket_for(active_state()), response(removed))

    assert updated.assigns.installed_ui_state.data.active_installation_id == nil

    assert [["ui-browser-close", %{"reason" => "active_generation_changed"}]] ==
             Phoenix.LiveView.Utils.get_push_events(updated)
  end

  test "reducer drives the initial load and delivered snapshot lifecycle" do
    state = InstalledUiNavigationEffects.new(%{organization_id: "org-1", target: :global})
    assert state.status == :initial

    {loading, [{:load, 1}]} = InstalledUiNavigationEffects.reduce(state, :load)
    assert loading.status == :loading
    assert loading.stream_generation == 1

    {ready, []} =
      InstalledUiNavigationEffects.reduce(
        loading,
        {:loaded, {:ok, %{"installations" => [@installation], "page" => %{}}}}
      )

    assert ready.status == :ready
    assert InstalledUiNavigationEffects.present(ready).state == :ready
  end

  test "reducer owns refresh, pagination, close, and termination effects" do
    state = active_state()

    {refreshed, [{:load, 0}]} = InstalledUiNavigationEffects.reduce(state, :refresh)
    assert refreshed.data.request_page_token == ""

    {paged, [{:load, 0}]} = InstalledUiNavigationEffects.reduce(state, :load_more)
    assert paged.data.request_page_token == "page-2"
    assert paged.data.loading_more

    {closed, [{:close, "user_closed"}]} = InstalledUiNavigationEffects.reduce(state, :close)
    assert closed.data.active_installation_id == nil

    {terminated, [{:close, "terminated"}]} =
      InstalledUiNavigationEffects.reduce(state, :terminate)

    assert terminated.status == :access_revoked
    assert InstalledUiNavigationEffects.present(terminated).state == :terminated
  end

  test "reducer validates launch before dispatching the transient handoff effect" do
    state = active_state()

    {unchanged, [{:handoff, @installation}]} =
      InstalledUiNavigationEffects.reduce(state, {:launch, "installation-1"})

    assert unchanged == state

    {revoked, [{:close, "access_revoked"}, {:flash, :error, _message}]} =
      InstalledUiNavigationEffects.reduce(
        InstalledUiNavigationEffects.complete(
          state,
          {:error, %Error{kind: :permission_denied, retryable: false}}
        ),
        {:launch, "installation-1"}
      )

    assert revoked.status == :access_revoked
  end

  test "synchronous handoff leaves an existing navigation task owned by the socket" do
    task = Task.async(fn -> Process.sleep(:infinity) end)

    stub = fn _channel, _request, _options ->
      {:ok,
       %CreateUiBrowserHandoffResponse{
         handoff_id: %OpaqueId{value: "40000000-0000-4000-8000-000000000004"},
         installation_id: %OpaqueId{value: "20000000-0000-4000-8000-000000000002"},
         generation_id: %OpaqueId{value: "30000000-0000-4000-8000-000000000003"},
         route: "docs"
       }}
    end

    socket =
      socket_for(active_state(),
        stub_call: stub,
        channel_provider: fn -> {:ok, %GRPC.Channel{}} end,
        ui_browser_options: [namespace: "ui.example.com", platform_host: "example.com", port: 443]
      )
      |> then(fn socket ->
        %{socket | assigns: Map.put(socket.assigns, :installed_ui_task, task)}
      end)

    updated = InstalledUiNavigation.launch(socket, "installation-1")
    assert updated.assigns.installed_ui_task == task

    assert [["ui-browser-launch", %{"installation_id" => "installation-1"}]] =
             Phoenix.LiveView.Utils.get_push_events(updated)

    Task.shutdown(task, :brutal_kill)
  end

  defp active_state do
    InstalledUiNavigationEffects.new(%{organization_id: "org-1", target: :global})
    |> InstalledUiNavigationEffects.complete({
      :ok,
      %{
        "installations" => [@installation],
        "page" => %{"next_page_token" => "page-2"}
      }
    })
    |> InstalledUiNavigationEffects.activate(@installation)
  end

  defp response(installation), do: {:ok, %{"installations" => [installation], "page" => %{}}}

  defp socket_for(state, handoff_options \\ []) do
    %Phoenix.LiveView.Socket{
      assigns: %{
        __changed__: %{},
        flash: %{},
        current_identity: identity(),
        installed_ui_state: state,
        installed_ui: InstalledUiNavigationEffects.present(state),
        installed_ui_task: nil
      },
      private: %{
        live_temp: %{},
        lifecycle: %Phoenix.LiveView.Lifecycle{},
        installed_ui_navigation_handoff_options: handoff_options
      }
    }
  end

  defp identity do
    %Identity{
      user_id: "50000000-0000-4000-8000-000000000005",
      issuer: "https://issuer.example",
      subject: "subject",
      display_name: "Test User",
      sid: "60000000-0000-4000-8000-000000000006"
    }
  end
end
