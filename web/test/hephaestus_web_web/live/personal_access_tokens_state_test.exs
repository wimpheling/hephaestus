defmodule HephaestusWebWeb.PersonalAccessTokensStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.PersonalAccessTokensState

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

  test "declares the complete lifecycle vocabulary" do
    assert @covered_statuses == PersonalAccessTokensState.statuses()
  end

  test "keeps one-time bearer material in an effect rather than page state" do
    state = PersonalAccessTokensState.new(%{})
    {loading, [{:load, 1}]} = PersonalAccessTokensState.reduce(state, :load)

    assert {ready, []} =
             PersonalAccessTokensState.reduce(loading, {
               :loaded,
               1,
               [%{"id" => "token-1", "label" => "laptop"}]
             })

    assert ready.status == :ready

    assert {issued, effects} =
             PersonalAccessTokensState.reduce(ready, {:issued, "one-time-sentinel"})

    assert {:reveal, "one-time-sentinel"} in effects
    refute inspect(issued) =~ "one-time-sentinel"
    assert {:load, 2} in effects
  end

  test "ignores stale metadata and reloads after revocation" do
    state = PersonalAccessTokensState.new(%{})
    {loading, _effects} = PersonalAccessTokensState.reduce(state, :load)

    assert PersonalAccessTokensState.reduce(loading, {:loaded, 0, []}) == {loading, []}
    assert {revoked, effects} = PersonalAccessTokensState.reduce(loading, :revoked)
    assert revoked.status == :submitting
    assert {:load, 2} in effects
  end
end
