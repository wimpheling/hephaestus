defmodule HephaestusWebWeb.RepositoryReleasesStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWebWeb.RepositoryReleasesState

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
    state = RepositoryReleasesState.new("repository-1")
    assert RepositoryReleasesState.statuses() == @covered_statuses

    {loading, [_effect]} =
      RepositoryReleasesState.reduce(state, {:load, %{}, "/repositories/repository-1/releases"})

    assert RepositoryReleasesState.reduce(loading, {:loaded, 0, {:error, :stale}}) ==
             {loading, []}

    assert {reconnecting, []} = RepositoryReleasesState.reduce(loading, :disconnected)
    assert RepositoryReleasesState.present(reconnecting).state == :reconnecting
  end
end
