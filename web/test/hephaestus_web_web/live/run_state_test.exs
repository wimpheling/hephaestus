defmodule HephaestusWebWeb.RunStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWeb.RPC.Error
  alias HephaestusWebWeb.ProductEventReducer
  alias HephaestusWebWeb.RunState

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

  test "covers lifecycle shape, reconnect, and stale results" do
    state = RunState.new("run-1")
    assert RunState.statuses() == @covered_statuses

    assert Map.take(state, [:status, :form, :error, :cursor, :stream_generation]) ==
             %{status: :initial, form: %{}, error: nil, cursor: nil, stream_generation: 0}

    {loading, [_effect]} = RunState.reduce(state, :load)
    assert RunState.reduce(loading, {:loaded, 0, {:error, :stale}}) == {loading, []}
    {reconnecting, []} = RunState.reduce(loading, :disconnected)
    assert RunState.present(reconnecting).state == :reconnecting

    {revoked, effects} =
      RunState.reduce(loading, {:loaded, loading.stream_generation, {:ok, false}})

    assert revoked.status == :access_revoked
    assert {:navigate, "/organizations"} in effects
  end

  test "a denied exact-scope watch revokes access instead of reconnecting forever" do
    state = RunState.new("missing-run") |> RunState.begin_watch()

    {revoked, effects} =
      ProductEventReducer.watch_ended(state, {:error, Error.local(:not_found)})

    assert revoked.status == :access_revoked
    assert effects == [{:navigate, :organizations}]
  end
end
