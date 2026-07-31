defmodule HephaestusWebWeb.RepositoryAgentsStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWebWeb.RepositoryAgentsState

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

  test "covers lifecycle, reconnect, and stale generations" do
    state = RepositoryAgentsState.new("repository-1")
    assert RepositoryAgentsState.statuses() == @covered_statuses

    {loading, [_effect]} =
      RepositoryAgentsState.reduce(state, {:load, %{}, "/repositories/repository-1/agents"})

    assert RepositoryAgentsState.reduce(loading, {:loaded, 0, {:error, :stale}}) == {loading, []}
    assert {reconnecting, []} = RepositoryAgentsState.reduce(loading, :disconnected)
    assert RepositoryAgentsState.present(reconnecting).state == :reconnecting
  end
end
