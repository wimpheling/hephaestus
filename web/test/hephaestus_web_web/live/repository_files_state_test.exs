defmodule HephaestusWebWeb.RepositoryFilesStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWebWeb.RepositoryFilesState

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

  test "covers lifecycle, reconnect, navigation, and stale generations" do
    state = RepositoryFilesState.new("repository-1")
    assert RepositoryFilesState.statuses() == @covered_statuses

    {loading, [_effect]} =
      RepositoryFilesState.reduce(state, {:load, %{}, "/repositories/repository-1/files"})

    assert RepositoryFilesState.reduce(loading, {:loaded, 0, {:error, :stale}}) == {loading, []}

    assert {_, [{:patch, "/repositories/repository-1/files?ref=main"}]} =
             RepositoryFilesState.reduce(loading, {:select_branch, "main"})

    assert {reconnecting, []} = RepositoryFilesState.reduce(loading, :disconnected)
    assert RepositoryFilesState.present(reconnecting).state == :reconnecting
  end
end
