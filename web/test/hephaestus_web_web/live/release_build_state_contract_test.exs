defmodule HephaestusWebWeb.ReleaseBuildStateContractTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.ReleaseState

  @build_state Module.concat(HephaestusWebWeb, "BuildState")
  @repository_builds_state Module.concat(HephaestusWebWeb, "RepositoryBuildsState")
  @lifecycle_statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]

  test "build and release pages expose loading, reconnecting, and denied states" do
    assert_lifecycle(ReleaseState, "release-1", :load)

    assert_lifecycle(
      @repository_builds_state,
      "repository-1",
      {:load, %{}, "/repositories/repository-1/builds"}
    )

    assert_lifecycle(@build_state, "build-1", :load)
  end

  test "build state keeps the three build actions distinct" do
    state_module = require_module(@build_state)
    statuses = apply(state_module, :statuses, [])

    assert Enum.all?(@lifecycle_statuses, &(&1 in statuses))

    ready = build_ready_state(state_module)

    for {event, status} <- [
          {:retry_attempt, :error},
          {:rebuild_for_verification, :error},
          {{:build_another_commit, "commit-2"}, :error}
        ] do
      {next, _effects} = apply(state_module, :reduce, [ready, event])
      assert next.status == status
    end
  end

  defp assert_lifecycle(state_module, resource_id, load_event) do
    state_module = require_module(state_module)
    assert Enum.all?(@lifecycle_statuses, &(&1 in apply(state_module, :statuses, [])))

    state = apply(state_module, :new, [resource_id])
    {loading, effects} = apply(state_module, :reduce, [state, load_event])

    assert loading.status == :loading
    assert effects != []
    assert apply(state_module, :present, [loading]).state == :loading

    {reconnecting, []} = apply(state_module, :reduce, [loading, :disconnected])
    assert reconnecting.status == :reconnecting
    assert apply(state_module, :present, [reconnecting]).state == :reconnecting

    generation = loading.stream_generation

    {denied, effects} =
      apply(state_module, :reduce, [loading, {:loaded, generation, {:error, :forbidden}}])

    assert denied.status == :access_revoked
    assert apply(state_module, :present, [denied]).state == :error
    assert Enum.any?(effects, &match?({:navigate, _}, &1))
  end

  defp build_ready_state(state_module) do
    state = apply(state_module, :new, ["build-1"])
    {loading, _effects} = apply(state_module, :reduce, [state, :load])

    {ready, _effects} =
      apply(state_module, :reduce, [
        loading,
        {:loaded, loading.stream_generation, {:ok, build()}}
      ])

    ready
  end

  defp build do
    %{
      "id" => "build-1",
      "repository_id" => "repository-1",
      "source_commit" => "commit-1",
      "source_ref" => "refs/heads/main",
      "state" => "failed"
    }
  end

  defp require_module(module) do
    assert Code.ensure_loaded?(module), "expected #{inspect(module)} to be implemented"
    module
  end
end
