defmodule HephaestusWebWeb.BuildStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.BuildState

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

  test "declares the complete lifecycle" do
    assert @covered_statuses == BuildState.statuses()
  end

  test "loads a build with repository scope and presents available fields" do
    state = BuildState.new("repository-1", "build-1")
    {loading, [{:load, generation, "repository-1", "build-1"}]} = BuildState.reduce(state, :load)

    assert loading.status == :loading

    assert BuildState.reduce(loading, {:loaded, generation - 1, {:ok, build_data()}}) ==
             {loading, []}

    {ready, []} = BuildState.reduce(loading, {:loaded, generation, {:ok, build_data()}})
    presentation = BuildState.present(ready)

    assert presentation.state == :ready
    assert presentation.build["id"] == "build-1"
    assert presentation.logs == ["a bounded log line"]

    assert presentation.timeline == [
             %{"to_state" => "succeeded", "reason" => "draft_release_created"}
           ]

    assert presentation.declared_artifacts == [%{"path" => "declared/output"}]
    assert presentation.produced_artifacts == [%{"path" => "produced/output"}]

    assert presentation.verifications == [
             %{
               "state" => "failed",
               "failure_code" => "manifest_mismatch",
               "expected_manifest" => [%{"path" => "declared/output"}],
               "actual_manifest" => [%{"path" => "produced/output"}]
             }
           ]

    assert presentation.destinations.repository == "/repositories/repository-1/builds"
  end

  test "maps reconnect, denied access, and the distinct build actions" do
    state = BuildState.new("repository-1", "build-1")
    {reconnecting, []} = BuildState.reduce(state, :disconnected)
    assert BuildState.present(reconnecting).state == :reconnecting

    {loading, [{:load, generation, _, _}]} = BuildState.reduce(state, :load)
    {revoked, effects} = BuildState.reduce(loading, {:loaded, generation, {:error, :forbidden}})
    assert revoked.status == :access_revoked
    assert {:navigate, "/organizations"} in effects

    for {event, operation} <- [
          {:retry_attempt, "BuildService.RetryBuild"},
          {{:build_another_commit, "commit-2"}, "BuildService.RequestBuild for another commit"}
        ] do
      {failed, [{:flash, :error, message}]} = BuildState.reduce(ready_state(), event)
      assert failed.status == :error
      assert message =~ operation
    end

    {verification, [{:action, _, :verification, "build-1"}]} =
      BuildState.reduce(ready_state(), :rebuild_for_verification)

    assert verification.status == :submitting
  end

  test "queues retry only for a failed build" do
    state = ready_state()
    failed = put_in(state.data.build["state"], "failed")
    {submitting, [{:action, _, :retry, "build-1"}]} = BuildState.reduce(failed, :retry_attempt)
    assert submitting.status == :submitting
  end

  defp ready_state do
    state = BuildState.new("repository-1", "build-1")
    {loading, _effects} = BuildState.reduce(state, :load)

    {ready, _effects} =
      BuildState.reduce(loading, {:loaded, loading.stream_generation, {:ok, build_data()}})

    ready
  end

  defp build_data do
    %{
      "id" => "build-1",
      "state" => "succeeded",
      "exit_code" => 0,
      "failure_code" => "",
      "logs" => ["a bounded log line"],
      "metrics" => [%{"name" => "duration", "value" => 1.0, "unit" => "seconds"}],
      "created_at" => ~U[2026-08-01 10:00:00Z],
      "updated_at" => ~U[2026-08-01 10:01:00Z],
      "timeline" => [%{"to_state" => "succeeded", "reason" => "draft_release_created"}],
      "declared_artifacts" => [%{"path" => "declared/output"}],
      "produced_artifacts" => [%{"path" => "produced/output"}],
      "verifications" => [
        %{
          "state" => "failed",
          "failure_code" => "manifest_mismatch",
          "expected_manifest" => [%{"path" => "declared/output"}],
          "actual_manifest" => [%{"path" => "produced/output"}]
        }
      ]
    }
  end
end
