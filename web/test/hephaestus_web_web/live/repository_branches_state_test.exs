defmodule HephaestusWebWeb.RepositoryBranchesStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWebWeb.RepositoryBranchesState

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
    state = RepositoryBranchesState.new("repository-1")
    assert RepositoryBranchesState.statuses() == @covered_statuses

    {loading, [_effect]} =
      RepositoryBranchesState.reduce(state, {:load, %{}, "/repositories/repository-1/branches"})

    assert RepositoryBranchesState.reduce(loading, {:loaded, 0, {:error, :stale}}) ==
             {loading, []}

    assert {reconnecting, []} = RepositoryBranchesState.reduce(loading, :disconnected)
    assert RepositoryBranchesState.present(reconnecting).state == :reconnecting
  end
end
