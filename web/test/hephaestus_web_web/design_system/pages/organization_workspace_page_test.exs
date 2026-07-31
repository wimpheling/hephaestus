defmodule HephaestusWebWeb.DesignSystem.Pages.OrganizationWorkspacePageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.OrganizationWorkspacePage
  alias HephaestusWebWeb.OrganizationWorkspaceState

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

  test "renders every declared state" do
    assert length(@covered_states) == 5
    organization = %{"id" => "organization-1", "name" => "Acme"}

    project = %{
      "id" => "project-1",
      "name" => "Forge",
      "last_activity_at" => nil,
      "repository_count" => 2,
      "instance_count" => 1,
      "run_count" => 4
    }

    for status <- OrganizationWorkspaceState.statuses() do
      state = OrganizationWorkspaceState.new(%{organization_id: "organization-1"})

      state = %{
        state
        | status: status,
          data: %{state.data | organization: organization, projects: [project]}
      }

      presentation = OrganizationWorkspaceState.present(state)

      html =
        render_component(&OrganizationWorkspacePage.organization_workspace_page/1,
          state: presentation.status,
          organization: presentation.organization,
          projects: presentation.projects
        )

      assert html != ""
    end

    assert length(@covered_statuses) == 8
  end

  test "renders project destinations in the ready state" do
    html =
      render_component(&OrganizationWorkspacePage.organization_workspace_page/1,
        state: :ready,
        organization: %{"id" => "organization-1", "name" => "Acme"},
        projects: [
          %{
            "id" => "project-1",
            "name" => "Forge",
            "last_activity_at" => nil,
            "repository_count" => 2,
            "instance_count" => 1,
            "run_count" => 4
          }
        ]
      )

    assert html =~ ~s(id="projects")
    assert html =~ ~s(href="/projects/project-1")
  end
end
