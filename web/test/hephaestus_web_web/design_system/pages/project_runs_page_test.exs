defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectRunsPageTest do
  use ExUnit.Case, async: true
  import Phoenix.LiveViewTest
  alias HephaestusWebWeb.DesignSystem.Pages.ProjectRunsPage
  alias HephaestusWebWeb.ProjectRunsState
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
    assert @covered_statuses == ProjectRunsState.statuses()

    for status <- @covered_statuses do
      state = ProjectRunsState.new(%{project_id: "project-1"})
      state = %{state | status: status, data: %{state.data | project: project(), runs: [run()]}}
      p = ProjectRunsState.present(state)

      html =
        render_component(&ProjectRunsPage.project_runs_page/1,
          state: p.status,
          project: p.project,
          project_id: p.project_id,
          item_count: p.item_count,
          runs: [{"project-run-run-1", run()}],
          organization_index_destination: "/organizations",
          organization_destination: "/organizations/org-1",
          run_destination: &"/runs/#{&1}"
        )

      assert html != ""
    end

    assert length(@covered_states) == 5
  end

  test "renders the established empty-run copy from a ready empty result" do
    state = ProjectRunsState.new(%{project_id: "project-1"})
    {loading, [:load]} = ProjectRunsState.reduce(state, {:load, 1})
    {ready, []} = ProjectRunsState.reduce(loading, {:loaded, 1, project(), []})
    presentation = ProjectRunsState.present(ready)

    assert presentation.status == :ready

    html =
      render_component(&ProjectRunsPage.project_runs_page/1,
        state: presentation.status,
        project: presentation.project,
        project_id: presentation.project_id,
        item_count: presentation.item_count,
        runs: [],
        organization_index_destination: "/organizations",
        organization_destination: "/organizations/org-1",
        run_destination: &"/runs/#{&1}"
      )

    assert html =~ "No exact runs have been created."
  end

  defp project,
    do: %{
      "id" => "project-1",
      "name" => "Forge",
      "organization_id" => "org-1",
      "organization_name" => "Acme"
    }

  defp run,
    do: %{
      "id" => "run-1",
      "instance_name" => "Cook",
      "repository_name" => "Source",
      "release_version" => "1.0.0",
      "outcome" => nil,
      "state" => "running"
    }
end
