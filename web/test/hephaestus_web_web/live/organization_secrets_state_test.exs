defmodule HephaestusWebWeb.OrganizationSecretsStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.OrganizationSecretsState

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

  test "rejects stale generations and never retains plaintext" do
    assert @covered_statuses == OrganizationSecretsState.statuses()
    state = OrganizationSecretsState.new(%{organization_id: "organization-1"})
    {loading, [:load]} = OrganizationSecretsState.reduce(state, {:load, 2})
    stale_secret = %{"id" => "secret-old", "value" => "must-not-persist"}

    {unchanged, []} =
      OrganizationSecretsState.reduce(
        loading,
        {:loaded, 1, %{"id" => "organization-1"}, [stale_secret], []}
      )

    assert unchanged == loading
    refute inspect(loading) =~ "must-not-persist"
    assert OrganizationSecretsState.stream_mode() == :none
  end

  test "refreshes from a finite snapshot after a receipt-confirmed command" do
    state = OrganizationSecretsState.new(%{organization_id: "organization-1"})
    receipt = %{committed_cursor: "cursor-1", event_id: "event-1", aggregate_version: 1}

    {refreshing, effects} =
      OrganizationSecretsState.reduce(state, {:command_succeeded, "Secret rotated.", receipt})

    assert refreshing.status == :submitting
    assert effects == [{:flash, :info, "Secret rotated."}, :snapshot]
  end
end
