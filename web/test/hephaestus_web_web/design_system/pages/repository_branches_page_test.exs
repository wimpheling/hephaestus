defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryBranchesPageTest do
  use ExUnit.Case, async: true
  import Phoenix.LiveViewTest
  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryBranchesPage
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

  test "renders every lifecycle visual and the branches route" do
    model = RepositoryPageFixtures.model()

    for state <- Map.values(@status_visual_states) do
      html =
        render_component(&RepositoryBranchesPage.repository_branches/1, %{
          state: state,
          model: model,
          branches: []
        })

      assert html =~ "repository-page-state" or html =~ "repository-branches"
    end

    assert MapSet.new(Map.values(@status_visual_states)) == MapSet.new(@covered_states)
  end

  test "keeps the empty branch page visible during background refreshes" do
    model = RepositoryPageFixtures.model()

    for state <- [:loading, :reconnecting] do
      html =
        render_component(&RepositoryBranchesPage.repository_branches/1, %{
          state: state,
          model: model,
          branches: []
        })

      assert html =~ "repository-branches"
      assert html =~ "No branches"
      refute html =~ "Repository unavailable"
    end
  end

  test "distinguishes initial loading from a real repository error" do
    model = Map.put(RepositoryPageFixtures.model(), :repository, nil)

    loading =
      render_component(&RepositoryBranchesPage.repository_branches/1, %{
        state: :loading,
        model: model,
        branches: []
      })

    failed =
      render_component(&RepositoryBranchesPage.repository_branches/1, %{
        state: :error,
        model: model,
        branches: []
      })

    assert loading =~ "Loading repository"
    refute loading =~ "Repository unavailable"
    assert failed =~ "Repository unavailable"
  end

  test "renders branches returned with native UTC datetimes" do
    model = %{RepositoryPageFixtures.model() | branches_empty?: false}

    html =
      render_component(&RepositoryBranchesPage.repository_branches/1, %{
        state: :ready,
        model: model,
        branches: [
          {"branch-main",
           %{
             name: "main",
             ref: "refs/heads/main",
             commit: "0123456789abcdef",
             subject: "Add relay",
             committed_at: ~U[2026-08-11 02:08:52Z]
           }}
        ]
      })

    assert html =~ "11 Aug 2026 · 02:08"
  end
end
