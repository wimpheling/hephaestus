defmodule HephaestusWebWeb.RepositoryFilesLiveTest do
  use ExUnit.Case, async: true

  alias Phoenix.Component
  alias HephaestusWebWeb.{RepositoryFilesLive, RepositoryFilesState}

  test "starts repository UI navigation after a successful effect-task load" do
    assert {:ok, mounted} =
             RepositoryFilesLive.mount(
               %{"repository_id" => "repository-1"},
               %{},
               disconnected_socket()
             )

    {loading, _effects} =
      RepositoryFilesState.reduce(
        mounted.assigns.page_state,
        {:load, %{}, "/repositories/repository-1/files"}
      )

    ref = make_ref()
    task = %Task{owner: self(), pid: self(), ref: ref, mfa: {__MODULE__, :noop, 0}}

    socket =
      mounted
      |> Component.assign(:page_state, loading)
      |> Component.assign(:presentation, RepositoryFilesState.present(loading))
      |> Component.assign(:effect_task, task)

    assert {:noreply, completed} =
             RepositoryFilesLive.handle_info(
               {ref, {:loaded, 1, {:ok, repository_snapshot()}}},
               socket
             )

    assert completed.assigns.page_state.status == :ready
    assert completed.assigns.installed_ui_state.data.organization_id == "organization-1"
    assert completed.assigns.installed_ui_state.data.target == {:repository, "repository-1"}
    assert completed.assigns.installed_ui.state == :loading
  end

  test "does not initialize repository UI navigation after an unsuccessful load" do
    assert {:ok, mounted} =
             RepositoryFilesLive.mount(
               %{"repository_id" => "repository-1"},
               %{},
               disconnected_socket()
             )

    {loading, _effects} =
      RepositoryFilesState.reduce(
        mounted.assigns.page_state,
        {:load, %{}, "/repositories/repository-1/files"}
      )

    ref = make_ref()
    task = %Task{owner: self(), pid: self(), ref: ref, mfa: {__MODULE__, :noop, 0}}

    socket =
      mounted
      |> Component.assign(:page_state, loading)
      |> Component.assign(:presentation, RepositoryFilesState.present(loading))
      |> Component.assign(:effect_task, task)

    assert {:noreply, completed} =
             RepositoryFilesLive.handle_info(
               {ref, {:loaded, 1, {:error, :unavailable}}},
               socket
             )

    assert completed.assigns.page_state.status == :error
    assert completed.assigns.installed_ui_state == nil
    assert completed.assigns.installed_ui.state == :loading
  end

  def noop, do: :ok

  defp repository_snapshot do
    %{
      repository_id: "repository-1",
      repository: %{
        "id" => "repository-1",
        "organization_id" => "organization-1",
        "project_id" => "project-1",
        "organization_name" => "Acme",
        "project_name" => "Forge",
        "name" => "Reference",
        "default_branch" => "refs/heads/main",
        "is_public" => false
      },
      remote_url: "https://forge.example/repository-1",
      selected_branch: nil,
      branch_options: [],
      branches: [],
      commits: [],
      builds: [],
      releases: [],
      attached_instances: [],
      tree: %{name: "", path: "", directories: [], files: [], file_count: 0},
      current_path: nil,
      file: nil,
      file_error: nil,
      params: %{},
      uri: "/repositories/repository-1/files"
    }
  end

  defp disconnected_socket do
    %Phoenix.LiveView.Socket{
      assigns: %{
        __changed__: %{},
        current_identity: %{subject: "identity-1"},
        flash: %{}
      },
      private: %{live_temp: %{}, lifecycle: %Phoenix.LiveView.Lifecycle{}}
    }
  end
end
