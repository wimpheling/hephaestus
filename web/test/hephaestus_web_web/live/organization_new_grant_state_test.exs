defmodule HephaestusWebWeb.OrganizationNewGrantStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.OrganizationNewGrantState

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

  test "reduces loaded grant choices into a presentation model" do
    assert @covered_statuses == OrganizationNewGrantState.statuses()
    state = OrganizationNewGrantState.new(%{organization_id: "organization-1"})
    organization = %{"id" => "organization-1", "name" => "Acme"}
    {ready, []} = OrganizationNewGrantState.reduce(state, {:loaded, organization, [], [], []})

    assert ready.status == :ready
    assert OrganizationNewGrantState.present(ready).organization == organization
    assert OrganizationNewGrantState.statuses() |> length() == 8
  end
end
