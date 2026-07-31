defmodule HephaestusWebWeb.OrganizationStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.OrganizationState

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

  test "constructs a presentation for every status" do
    for status <- @covered_statuses do
      state = %{OrganizationState.new(%{}) | status: status}

      assert OrganizationState.present(state).status in [
               :loading,
               :empty,
               :error,
               :reconnecting,
               :ready
             ]
    end
  end
end
