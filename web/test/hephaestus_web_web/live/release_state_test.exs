defmodule HephaestusWebWeb.ReleaseStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.ReleaseState

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

  test "exposes the complete lifecycle contract" do
    state = ReleaseState.new("release-1")

    assert ReleaseState.statuses() == @covered_statuses
    assert state.status == :initial
    assert state.data == %{release_id: "release-1"}
    assert state.form == %{}
    assert state.error == nil
    assert state.cursor == nil
    assert state.stream_generation == 0
  end

  test "loads through an effect and rejects stale generations" do
    state = ReleaseState.new("release-1")
    {loading, [{:load, generation, "release-1"}]} = ReleaseState.reduce(state, :load)

    assert loading.status == :loading
    assert loading.stream_generation == generation

    assert ReleaseState.reduce(loading, {:loaded, generation - 1, {:ok, release()}}) ==
             {loading, []}

    {ready, []} = ReleaseState.reduce(loading, {:loaded, generation, {:ok, release()}})
    presentation = ReleaseState.present(ready)

    assert ready.status == :ready
    assert presentation.state == :ready
    assert presentation.release["id"] == "release-1"
  end

  test "presents reconnecting and revoked access without domain work" do
    state = ReleaseState.new("release-1")
    {reconnecting, []} = ReleaseState.reduce(state, :disconnected)

    assert ReleaseState.present(reconnecting).state == :reconnecting

    {loading, _effects} = ReleaseState.reduce(state, :load)
    generation = loading.stream_generation

    {revoked, [{:navigate, "/organizations"}]} =
      ReleaseState.reduce(loading, {:loaded, generation, {:error, :forbidden}})

    assert revoked.status == :access_revoked
    assert ReleaseState.present(revoked).state == :error
  end

  defp release do
    %{
      "id" => "release-1",
      "repository_id" => "repository-1",
      "organization_id" => "organization-1",
      "project_id" => "project-1",
      "source_ref" => "refs/heads/main",
      "artifacts" => [],
      "agents" => []
    }
  end
end
