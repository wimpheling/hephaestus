defmodule HephaestusWebWeb.ProjectSettingsStateTest do
  use ExUnit.Case, async: true
  alias HephaestusWebWeb.ProjectSettingsState

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

  test "uses the common shape and never retains secret plaintext" do
    assert @covered_statuses == ProjectSettingsState.statuses()
    state = ProjectSettingsState.new(%{project_id: "project-1"})

    assert Map.keys(Map.from_struct(state)) |> Enum.sort() == [
             :cursor,
             :data,
             :error,
             :form,
             :status,
             :stream_generation
           ]

    refute inspect(state) =~ "plaintext"
    assert ProjectSettingsState.accept_import_message() == "Live secret reference accepted."
    assert ProjectSettingsState.create_secret_message() == "Secret encrypted and stored."
    assert ProjectSettingsState.grant_message() == "Bounded secret grant offered."
    assert ProjectSettingsState.stream_mode() == :none
  end

  test "refreshes from a finite snapshot after a receipt-confirmed command" do
    state = ProjectSettingsState.new(%{project_id: "project-1"})
    receipt = %{committed_cursor: "cursor-1", event_id: "event-1", aggregate_version: 1}

    {refreshing, effects} =
      ProjectSettingsState.reduce(state, {:command_succeeded, "Secret rotated.", receipt})

    assert refreshing.status == :submitting
    assert effects == [{:flash, :info, "Secret rotated."}, :snapshot]
  end
end
