defmodule HephaestusWebWeb.ProjectNewStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.ProjectNewState

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

  test "declares the complete lifecycle and keeps form state typed" do
    assert @covered_statuses == ProjectNewState.statuses()
    state = ProjectNewState.new(%{organization_id: "organization-1"})

    assert state.form == %{"name" => "", "description" => ""}

    assert Map.keys(Map.from_struct(state)) |> Enum.sort() ==
             [:cursor, :data, :error, :form, :status, :stream_generation]

    {loading, [:load]} = ProjectNewState.reduce(state, :load)
    assert loading.status == :loading

    {ready, []} =
      ProjectNewState.reduce(loading, {:loaded, {:ok, %{"id" => "organization-1"}}})

    assert ProjectNewState.present(ready).state == :ready
  end

  test "turns service results into navigation or useful errors" do
    state = ProjectNewState.new(%{organization_id: "organization-1"})

    {ready, [{:flash, :info, _}, {:navigate, "/projects/project-1"}]} =
      ProjectNewState.reduce(state, {:created, {:ok, %{"project_id" => "project-1"}}})

    assert ready.status == :ready
    {failed, [{:flash, :error, message}]} = ProjectNewState.reduce(state, {:failed, :timeout})
    assert failed.status == :error
    assert message != ""
  end

  test "keeps the project description in the typed creation attributes" do
    state = ProjectNewState.new(%{organization_id: "organization-1"})
    attributes = %{"name" => "Forge", "description" => "Agent release workspace"}

    {submitting, []} = ProjectNewState.reduce(state, :submitting)
    {submitted, []} = ProjectNewState.reduce(submitting, {:form, attributes})

    assert submitted.form == attributes
  end
end
