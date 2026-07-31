defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectPageTest do
  use ExUnit.Case, async: true
  import Phoenix.LiveViewTest
  alias HephaestusWebWeb.DesignSystem.Pages.ProjectPage
  alias HephaestusWebWeb.ProjectState
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

  test "renders presentation derived from every state status" do
    assert @covered_statuses == ProjectState.statuses()

    for status <- @covered_statuses do
      state = ProjectState.new(%{project_id: "project-1"})

      state = %{
        state
        | status: status,
          data: %{state.data | project: project(), repositories: [repository()]}
      }

      presentation = ProjectState.present(state)

      html =
        render_component(&ProjectPage.project_page/1,
          state: presentation.status,
          project: presentation.project,
          project_id: "project-1",
          item_count: presentation.item_count,
          repositories: [{"project-repository-repository-1", repository()}],
          organization_index_destination: "/organizations",
          organization_destination: "/organizations/org-1",
          repository_destination: &"/repositories/#{&1}"
        )

      assert html != ""
    end

    assert length(@covered_states) == 5
  end

  defp project,
    do: %{
      "id" => "project-1",
      "name" => "Forge",
      "organization_id" => "org-1",
      "organization_name" => "Acme"
    }

  defp repository,
    do: %{
      "id" => "repository-1",
      "name" => "Source",
      "is_public" => false,
      "default_branch" => "refs/heads/main",
      "attachment_count" => 1,
      "run_count" => 2
    }
end
