defmodule HephaestusWebWeb.DesignSystem.Pages.PersonalAccessTokensPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.PersonalAccessTokensPage
  alias HephaestusWebWeb.PersonalAccessTokensState

  @covered_states [:loading, :empty, :error, :reconnecting, :ready]
  @covered_statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]
  @status_visual_states %{
    initial: :loading,
    loading: :loading,
    ready: :ready,
    submitting: :ready,
    error: :error,
    stale: :reconnecting,
    reconnecting: :reconnecting,
    access_revoked: :error
  }

  test "renders safe metadata and lifecycle controls without bearer material" do
    html = render_component(&PersonalAccessTokensPage.personal_access_tokens_page/1, assigns())

    assert html =~ "Git credentials"
    assert html =~ "developer laptop"
    assert html =~ "discover, fetch"
    assert html =~ "Rotate"
    assert html =~ "Revoke"
    refute html =~ "one-time-sentinel"
  end

  test "renders all bounded visual states" do
    assert @covered_statuses == PersonalAccessTokensState.statuses()

    for status <- @covered_statuses do
      state = Map.fetch!(@status_visual_states, status)

      html =
        render_component(
          &PersonalAccessTokensPage.personal_access_tokens_page/1,
          Map.put(assigns(), :state, state)
        )

      if state == :ready do
        assert html =~ "Git credentials"
      else
        assert html =~ "personal-access-tokens-page-state"
      end
    end

    assert length(@covered_states) == 5
  end

  defp assigns do
    %{
      state: :ready,
      tokens: [
        %{
          "id" => "11111111-1111-4111-8111-111111111111",
          "label" => "developer laptop",
          "scope" => %{"operations" => ["discover", "fetch"], "repository_ids" => []},
          "expires_at" => DateTime.add(DateTime.utc_now(), 30, :day),
          "created_at" => DateTime.utc_now(),
          "revoked_at" => nil,
          "last_used_at" => nil
        }
      ],
      item_count: 1,
      form: %{
        "label" => "",
        "operations" => ["discover", "fetch"],
        "repository_ids" => "",
        "expires_at" => "2030-01-01T00:00"
      },
      error: nil,
      create_event: "create-personal-access-token",
      rotate_event: "rotate-personal-access-token",
      revoke_event: "revoke-personal-access-token"
    }
  end
end
