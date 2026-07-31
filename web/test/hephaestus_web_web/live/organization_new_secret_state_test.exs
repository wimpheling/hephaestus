defmodule HephaestusWebWeb.OrganizationNewSecretStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.OrganizationNewSecretState

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

  test "keeps the common state shape without a plaintext field" do
    assert @covered_statuses == OrganizationNewSecretState.statuses()
    state = OrganizationNewSecretState.new(%{organization_id: "organization-1"})

    assert Map.keys(Map.from_struct(state)) |> Enum.sort() ==
             [:cursor, :data, :error, :form, :status, :stream_generation]

    refute Map.has_key?(state, :value)
    assert OrganizationNewSecretState.stream_mode() == :none
  end
end
