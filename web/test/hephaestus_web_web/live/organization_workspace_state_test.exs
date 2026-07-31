defmodule HephaestusWebWeb.OrganizationWorkspaceStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.OrganizationWorkspaceState

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

  test "uses the complete page-state contract" do
    assert @covered_statuses == OrganizationWorkspaceState.statuses()
    state = OrganizationWorkspaceState.new(%{organization_id: "organization-1"})

    assert OrganizationWorkspaceState.statuses() == [
             :initial,
             :loading,
             :ready,
             :submitting,
             :error,
             :stale,
             :reconnecting,
             :access_revoked
           ]

    assert Map.keys(Map.from_struct(state)) |> Enum.sort() ==
             [:cursor, :data, :error, :form, :status, :stream_generation]
  end

  test "reduces loading and loaded data without effects in render" do
    state = OrganizationWorkspaceState.new(%{organization_id: "organization-1"})
    {loading, [:load]} = OrganizationWorkspaceState.reduce(state, :load)
    organization = %{"id" => "organization-1", "name" => "Acme"}
    projects = [%{"id" => "project-1", "name" => "Forge"}]

    {ready, []} =
      OrganizationWorkspaceState.reduce(loading, {:loaded, organization, projects})

    assert ready.status == :ready
    assert OrganizationWorkspaceState.present(ready).organization == organization
    assert OrganizationWorkspaceState.present(ready).projects == projects
  end
end
