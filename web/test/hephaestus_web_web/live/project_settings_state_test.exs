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
  end
end
