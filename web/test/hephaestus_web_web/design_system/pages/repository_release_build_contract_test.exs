defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryReleaseBuildContractTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.{
    RepositoryAgentsPage,
    RepositoryBranchesPage,
    RepositoryCommitsPage,
    RepositoryFilesPage,
    RepositoryBuildsPage,
    RepositoryReleasesPage
  }

  alias HephaestusWebWeb.RepositoryPageFixtures

  @repository_id "repository-1"
  @build_page Module.concat(HephaestusWebWeb.DesignSystem.Pages, "BuildPage")
  @repository_tabs [
    {"Files", "/repositories/#{@repository_id}/files"},
    {"Commits", "/repositories/#{@repository_id}/commits"},
    {"Branches", "/repositories/#{@repository_id}/branches"},
    {"Builds", "/repositories/#{@repository_id}/builds"},
    {"Releases", "/repositories/#{@repository_id}/releases"},
    {"Agents", "/repositories/#{@repository_id}/agents"}
  ]

  test "every repository route exposes files, commits, branches, builds, releases, and agents" do
    model = RepositoryPageFixtures.model()

    for {page, page_kind} <- [
          {&RepositoryFilesPage.repository_files/1, :files},
          {&RepositoryCommitsPage.repository_commits/1, :commits},
          {&RepositoryBuildsPage.repository_builds/1, :builds},
          {&RepositoryBranchesPage.repository_branches/1, :branches},
          {&RepositoryReleasesPage.repository_releases/1, :releases},
          {&RepositoryAgentsPage.repository_agents/1, :agents}
        ] do
      html = render_component(page, page_assigns(page_kind, model))

      for {label, destination} <- @repository_tabs do
        assert html =~ label
        assert html =~ ~s(href="#{destination}")
      end
    end
  end

  test "build detail names each action by its actual semantics" do
    build_page = require_module(@build_page)

    html =
      render_component(
        fn assigns -> apply(build_page, :build, [assigns]) end,
        build_assigns()
      )

    assert html =~ "Retry attempt"
    assert html =~ "Rebuild for verification"
    assert html =~ "Build another commit"
    refute html =~ ~r/>Rebuild\s*</
    refute html =~ ~s(phx-click="rebuild")
  end

  defp page_assigns(:files, model) do
    %{
      state: :ready,
      model: model,
      branch_form: Phoenix.Component.to_form(model.browse_form, as: :browse),
      select_branch_event: "select-branch"
    }
  end

  defp page_assigns(:commits, model) do
    %{
      state: :ready,
      model: model,
      commits: [],
      branch_form: Phoenix.Component.to_form(model.browse_form, as: :browse),
      select_branch_event: "select-branch"
    }
  end

  defp page_assigns(:builds, model) do
    model =
      Map.merge(model, %{
        builds_empty?: true,
        builds_unavailable?: false,
        build_request_form:
          Phoenix.Component.to_form(
            %{
              "source_commit" => "",
              "build_definition_hash" => "",
              "configuration_hash" => ""
            },
            as: :build
          )
      })

    %{
      state: :ready,
      model: model,
      builds: [],
      build_request_form: model.build_request_form,
      request_event: "request-build"
    }
  end

  defp page_assigns(:branches, model),
    do: %{state: :ready, model: model, branches: []}

  defp page_assigns(:releases, model),
    do: %{state: :ready, model: model, releases: []}

  defp page_assigns(:agents, model),
    do: %{state: :ready, model: model, attachments: []}

  defp build_assigns do
    %{
      state: :ready,
      build: %{
        "id" => "build-1",
        "state" => "failed",
        "source_commit" => "commit-1",
        "source_ref" => "refs/heads/main",
        "configuration_hash" => "configuration-hash",
        "builder_image" => "fedora-minimal@sha256:builder",
        "exit_code" => 1,
        "failure_code" => "agent_failed"
      },
      timeline: [],
      logs: [],
      metrics: [],
      artifacts: [],
      retry_event: "retry-attempt",
      verification_rebuild_event: "rebuild-for-verification",
      another_commit_event: "build-another-commit"
    }
  end

  defp require_module(module) do
    assert Code.ensure_loaded?(module), "expected #{inspect(module)} to be implemented"
    module
  end
end
