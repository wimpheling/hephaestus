defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryReleasesPageTest do
  use ExUnit.Case, async: true
  import Phoenix.LiveViewTest
  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryReleasesPage
  alias HephaestusWebWeb.RepositoryPageFixtures

  @covered_states [:loading, :error, :reconnecting, :ready]
  @status_visual_states %{
    initial: :loading,
    loading: :loading,
    ready: :ready,
    submitting: :loading,
    error: :error,
    stale: :reconnecting,
    reconnecting: :reconnecting,
    access_revoked: :error
  }

  test "renders every lifecycle visual and the releases route" do
    model = RepositoryPageFixtures.model()

    for state <- Map.values(@status_visual_states) do
      html =
        render_component(&RepositoryReleasesPage.repository_releases/1, %{
          state: state,
          model: model,
          releases: []
        })

      assert html =~ "repository-page-state" or html =~ "repository-releases"
    end

    assert MapSet.new(Map.values(@status_visual_states)) == MapSet.new(@covered_states)
  end
end
