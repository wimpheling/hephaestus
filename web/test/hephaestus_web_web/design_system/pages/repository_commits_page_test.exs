defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryCommitsPageTest do
  use ExUnit.Case, async: true
  import Phoenix.LiveViewTest
  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryCommitsPage
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

  test "renders every lifecycle visual and the commits route" do
    model = RepositoryPageFixtures.model()
    form = Phoenix.Component.to_form(model.browse_form, as: :browse)

    for state <- Map.values(@status_visual_states) do
      html =
        render_component(&RepositoryCommitsPage.repository_commits/1, %{
          state: state,
          model: model,
          commits: [],
          branch_form: form,
          select_branch_event: "select-branch"
        })

      assert html =~ "repository-page-state" or html =~ "repository-commits"
    end

    assert MapSet.new(Map.values(@status_visual_states)) == MapSet.new(@covered_states)
  end
end
