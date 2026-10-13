defmodule HephaestusWebWeb.OrganizationNewSecretClearTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.{OrganizationNewSecretLive, OrganizationNewSecretState}

  test "command failure clears plaintext without changing metadata or error semantics" do
    original = socket()

    {:noreply, updated} =
      OrganizationNewSecretLive.handle_async(
        :command,
        {:ok, {:failed, :denied, "Command was denied or failed validation."}},
        original
      )

    assert_clear(updated)
    assert updated.assigns.page_state == original.assigns.page_state
    assert updated.assigns.flash["error"] == "Command was denied or failed validation."
  end

  test "task exit clears plaintext while preserving the unavailable error" do
    {:noreply, updated} =
      OrganizationNewSecretLive.handle_async(:command, {:exit, :unavailable}, socket())

    assert_clear(updated)
    assert updated.assigns.flash["error"] == "Command service is temporarily unavailable."
  end

  test "success clears plaintext before navigating to the secret list" do
    {:noreply, updated} =
      OrganizationNewSecretLive.handle_async(
        :command,
        {:ok, {:command_succeeded, "Organization secret encrypted and stored."}},
        socket()
      )

    assert_clear(updated)
    assert {:live, :redirect, %{to: "/organizations/organization-1/secrets"}} = updated.redirected
  end

  test "access revocation clears plaintext before navigating away" do
    {:noreply, updated} =
      OrganizationNewSecretLive.handle_async(:load, {:ok, {:access_revoked, :denied}}, socket())

    assert_clear(updated)
    assert updated.assigns.page_state.status == :access_revoked
    assert {:live, :redirect, %{to: "/organizations"}} = updated.redirected
  end

  defp assert_clear(socket) do
    assert [["organization-secret-clear", %{}]] == Phoenix.LiveView.Utils.get_push_events(socket)
  end

  defp socket do
    state = OrganizationNewSecretState.new(%{organization_id: "organization-1"})

    {state, []} =
      OrganizationNewSecretState.reduce(state, {:loaded, %{"id" => "organization-1"}, []})

    %Phoenix.LiveView.Socket{
      assigns: %{__changed__: %{}, flash: %{}, page_state: state},
      private: %{live_temp: %{}}
    }
  end
end
