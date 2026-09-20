defmodule HephaestusWebWeb.InstalledUiNavigationLiveTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.RPC.Error
  alias HephaestusWebWeb.{InstalledUiNavigationLive, InstalledUiNavigationState}

  @installation %{
    "installation_id" => "installation-1",
    "generation_id" => "generation-1",
    "lifecycle" => "enabled",
    "launchable" => true,
    "route_base" => "docs"
  }

  test "closes the active frame as access revoked after a denied refresh" do
    socket = socket_for(InstalledUiNavigationState.load_more(active_state()))

    updated =
      InstalledUiNavigationLive.complete(
        socket,
        {:error, %Error{kind: :permission_denied, retryable: false}}
      )

    assert updated.assigns.installed_ui_state.status == :access_revoked

    assert [["ui-browser-close", %{"reason" => "access_revoked"}]] ==
             Phoenix.LiveView.Utils.get_push_events(updated)
  end

  test "closes the active frame as unavailable after an error refresh" do
    socket = socket_for(InstalledUiNavigationState.load_more(active_state()))

    updated =
      InstalledUiNavigationLive.complete(
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
      InstalledUiNavigationLive.complete(socket_for(active_state()), response(replacement))

    assert updated.assigns.installed_ui_state.active_generation_id == nil

    assert [["ui-browser-close", %{"reason" => "active_generation_changed"}]] ==
             Phoenix.LiveView.Utils.get_push_events(updated)
  end

  test "closes the active frame when its installation is disabled" do
    disabled = %{@installation | "lifecycle" => "disabled", "launchable" => false}
    updated = InstalledUiNavigationLive.complete(socket_for(active_state()), response(disabled))

    assert updated.assigns.installed_ui_state.active_installation_id == nil

    assert [["ui-browser-close", %{"reason" => "active_generation_changed"}]] ==
             Phoenix.LiveView.Utils.get_push_events(updated)
  end

  test "closes the active frame when its installation is removed" do
    removed = %{@installation | "lifecycle" => "removed", "launchable" => false}
    updated = InstalledUiNavigationLive.complete(socket_for(active_state()), response(removed))

    assert updated.assigns.installed_ui_state.active_installation_id == nil

    assert [["ui-browser-close", %{"reason" => "active_generation_changed"}]] ==
             Phoenix.LiveView.Utils.get_push_events(updated)
  end

  defp active_state do
    "org-1"
    |> InstalledUiNavigationState.new(:global)
    |> InstalledUiNavigationState.complete({
      :ok,
      %{
        "installations" => [@installation],
        "page" => %{"next_page_token" => "page-2"}
      }
    })
    |> InstalledUiNavigationState.activate(@installation)
  end

  defp response(installation), do: {:ok, %{"installations" => [installation], "page" => %{}}}

  defp socket_for(state) do
    %Phoenix.LiveView.Socket{
      assigns: %{
        __changed__: %{},
        current_identity: %{subject: "identity-1"},
        installed_ui_state: state,
        installed_ui: InstalledUiNavigationState.present(state),
        installed_ui_task: nil
      },
      private: %{live_temp: %{}, lifecycle: %Phoenix.LiveView.Lifecycle{}}
    }
  end
end
