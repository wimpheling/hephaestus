defmodule HephaestusWebWeb.RepositoryLiveSupportTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.{
    RepositoryBranchesState,
    RepositoryBuildsState,
    RepositoryCommitsState,
    RepositoryFilesState,
    RepositoryLiveSupport,
    RepositoryReleasesState,
    RepositoryAgentsState
  }

  test "does not duplicate route RPC work during the disconnected render" do
    socket = %Phoenix.LiveView.Socket{
      assigns: %{
        __changed__: %{},
        current_identity: :identity,
        effect_task: nil
      }
    }

    effect =
      {:load, 1, :files, "repository-1", %{}, "/repositories/repository-1/files"}

    updated = RepositoryLiveSupport.start_effect(socket, RepositoryFilesState, effect)

    assert updated.assigns.effect_task == nil
  end

  test "browse routes are snapshot-only and ignore watch reload effects" do
    for state_module <- [
          RepositoryFilesState,
          RepositoryCommitsState,
          RepositoryBranchesState,
          RepositoryBuildsState,
          RepositoryReleasesState,
          RepositoryAgentsState
        ] do
      assert state_module.stream_mode() == :none
    end

    socket = %Phoenix.LiveView.Socket{
      assigns: %{
        __changed__: %{},
        stream_mode: :none,
        snapshot_task: nil,
        watch_task: nil
      }
    }

    updated =
      RepositoryLiveSupport.apply_effects(socket, RepositoryFilesState, [:snapshot, :replace_watch])

    assert updated.assigns.snapshot_task == nil
    assert updated.assigns.watch_task == nil
  end
end
