defmodule HephaestusWebWeb.RepositoryCommitsStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWebWeb.RepositoryCommitsState

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
    state = RepositoryCommitsState.new("repository-1")
    assert RepositoryCommitsState.statuses() == @covered_statuses

    {loading, [_effect]} =
      RepositoryCommitsState.reduce(state, {:load, %{}, "/repositories/repository-1/commits"})

    assert RepositoryCommitsState.reduce(loading, {:loaded, 0, {:error, :stale}}) == {loading, []}

    assert {_, [{:patch, "/repositories/repository-1/commits?ref=main"}]} =
             RepositoryCommitsState.reduce(loading, {:select_branch, "main"})

    assert {reconnecting, []} = RepositoryCommitsState.reduce(loading, :disconnected)
    assert RepositoryCommitsState.present(reconnecting).state == :reconnecting
  end
end
