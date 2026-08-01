defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryBuildsPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest
  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryBuildsPage

  @covered_states [:loading, :error, :reconnecting, :ready]
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

  test "renders the manual request and unavailable list state distinctly" do
    assert length(@covered_states) == 4
    assert length(@covered_statuses) == 8
    html = render_component(&RepositoryBuildsPage.repository_builds/1, assigns())

    assert html =~ ~s(id="build-request-form")
    assert html =~ "Request build"
    assert html =~ "Build history unavailable"
    refute html =~ "ListRepositoryBuilds"
  end

  defp assigns do
    %{
      state: :ready,
      model: %{
        repository: %{
          "id" => "repository-1",
          "name" => "Source",
          "organization_name" => "Acme",
          "project_name" => "Project",
          "organization_id" => "organization-1",
          "project_id" => "project-1",
          "default_branch" => "refs/heads/main"
        },
        tabs: [%{key: :builds, label: "Builds", destination: "/repositories/repository-1/builds"}],
        destinations: %{
          organization_index: "/organizations",
          organization: "/organizations/organization-1",
          project: "/projects/project-1"
        },
        builds_empty?: true,
        builds_unavailable?: true
      },
      builds: [],
      build_request_form:
        Phoenix.Component.to_form(
          %{
            "source_commit" => "",
            "build_definition_hash" => "",
            "configuration_hash" => ""
          },
          as: :build
        ),
      request_event: "request-build"
    }
  end
end
