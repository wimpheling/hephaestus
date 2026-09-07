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

  test "accepts optional empty rows and rejects malformed brokered copy IDs" do
    assert AgentInstanceState.validate_brokered_rule_copies([
             %{"source_rule_id" => "", "candidate_rule_id" => ""}
           ]) == {:ok, []}

    assert AgentInstanceState.validate_brokered_rule_copies([
             %{
               "source_rule_id" => "00000000-0000-4000-8000-000000000001",
               "candidate_rule_id" => "not-a-uuid"
             }
           ]) == {:error, :invalid_brokered_rule_copies}
  end

  test "adds and removes bounded generic brokered copy rows" do
    state = AgentInstanceState.new("instance-1")

    {one, []} =
      AgentInstanceState.reduce(state, {:interaction, "add-brokered-rule-copy", %{}})

    {two, []} =
      AgentInstanceState.reduce(one, {:interaction, "add-brokered-rule-copy", %{}})

    assert AgentInstanceState.present(two).brokered_rule_copy_count == 2

    {one_again, []} =
      AgentInstanceState.reduce(two, {:interaction, "remove-brokered-rule-copy", %{}})

    assert AgentInstanceState.present(one_again).brokered_rule_copy_count == 1
  end
end
