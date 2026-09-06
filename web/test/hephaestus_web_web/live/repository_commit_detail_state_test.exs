defmodule HephaestusWebWeb.RepositoryCommitDetailStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.RepositoryCommitDetailState

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
    state = RepositoryCommitDetailState.new("repository-1")
    assert RepositoryCommitDetailState.statuses() == @covered_statuses

    {loading, [_effect]} =
      RepositoryCommitDetailState.reduce(
        state,
        {:load, %{"commit" => "0123456789abcdef"},
         "/repositories/repository-1/commits/0123456789abcdef"}
      )

    assert RepositoryCommitDetailState.reduce(loading, {:loaded, 0, {:error, :stale}}) ==
             {loading, []}

    assert {reconnecting, []} = RepositoryCommitDetailState.reduce(loading, :disconnected)
    assert RepositoryCommitDetailState.present(reconnecting).state == :reconnecting
  end
end
