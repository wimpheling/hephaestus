defmodule HephaestusWebWeb.AgentInstanceStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWebWeb.AgentInstanceState

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
    state = AgentInstanceState.new("instance-1")
    assert AgentInstanceState.statuses() == @covered_statuses

    assert Map.take(state, [:status, :form, :error, :cursor, :stream_generation]) ==
             %{status: :initial, form: state.form, error: nil, cursor: nil, stream_generation: 0}

    {loading, [_effect]} = AgentInstanceState.reduce(state, :load)
    assert AgentInstanceState.reduce(loading, {:loaded, 0, {:error, :stale}}) == {loading, []}
    {reconnecting, []} = AgentInstanceState.reduce(loading, :disconnected)
    assert AgentInstanceState.present(reconnecting).state == :reconnecting
  end

  test "waits for the command receipt to arrive on the active watch" do
    state = AgentInstanceState.new("instance-1")

    {submitting, [{:command, generation, "bind-secret", _params}]} =
      AgentInstanceState.reduce(state, {:interaction, "bind-secret", %{"binding" => %{}}})

    assert submitting.status == :submitting
    assert submitting.stream_generation == generation

    receipt = %{committed_cursor: "cursor-2", event_id: "event-2", aggregate_version: 2}

    {waiting, effects} =
      AgentInstanceState.reduce(
        submitting,
        {:command_completed, generation, {:ok, receipt, "Secret binding activated"}}
      )

    assert waiting.status == :stale
    assert waiting.stream_generation == generation
    assert effects == [{:flash, :info, "Secret binding activated"}]
    assert waiting.data.watch_required_receipts == [receipt]
  end
end
