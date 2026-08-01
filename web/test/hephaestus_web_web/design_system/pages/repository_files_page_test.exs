defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryFilesPageTest do
  use ExUnit.Case, async: true
  import Phoenix.LiveViewTest
  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryFilesPage
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

  test "renders every lifecycle visual and the files route" do
    model = RepositoryPageFixtures.model()
    form = Phoenix.Component.to_form(model.browse_form, as: :browse)

    for state <- Map.values(@status_visual_states) do
      html =
        render_component(&RepositoryFilesPage.repository_files/1, %{
          state: state,
          model: model,
          branch_form: form,
          select_branch_event: "select-branch"
        })

      assert html =~ "repository-page-state" or html =~ "repository-files" or
               html =~ "repository-empty-push"
    end

    assert MapSet.new(Map.values(@status_visual_states)) == MapSet.new(@covered_states)
  end

  test "renders credential-free first-push instructions for an empty repository" do
    model =
      RepositoryPageFixtures.model()
      |> Map.put(:remote_url, "https://forge.example/repository-1")
      |> Map.put(:default_branch, "trunk")

    html =
      render_component(&RepositoryFilesPage.repository_files/1, %{
        state: :ready,
        model: model,
        branch_form: Phoenix.Component.to_form(model.browse_form, as: :browse),
        select_branch_event: "select-branch"
      })

    assert html =~ "Push your first commit"
    assert html =~ "https://forge.example/repository-1"
    assert html =~ "push -u origin trunk"
    assert html =~ "HEPHAESTUS_GIT_TOKEN"
    assert html =~ "agent.toml"
    refute html =~ "Bearer ey"
  end
end
