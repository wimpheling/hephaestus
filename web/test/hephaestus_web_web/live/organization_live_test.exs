defmodule HephaestusWebWeb.OrganizationLiveTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.RPC.Error
  alias HephaestusWebWeb.OrganizationLive

  alias HephaestusWebWeb.{
    OrganizationSecretsLive,
    OrganizationSecretsState,
    OrganizationState,
    OrganizationWorkspaceLive,
    OrganizationWorkspaceState,
    ProjectAgentsLive,
    ProjectAgentsState,
    ProjectLive,
    ProjectRunsLive,
    ProjectRunsState,
    ProjectSettingsLive,
    ProjectSettingsState,
    ProjectState
  }

  test "initializes the organization stream during disconnected mount" do
    assert {:ok, mounted} = OrganizationLive.mount(%{}, %{}, disconnected_socket())
    assert %Phoenix.LiveView.LiveStream{} = mounted.assigns.streams.organizations
    assert Enum.empty?(mounted.assigns.streams.organizations.inserts)
  end

  test "initializes every project stream during disconnected mount" do
    for {view, stream_name} <- [
          {ProjectLive, :repositories},
          {ProjectAgentsLive, :instances},
          {ProjectRunsLive, :runs},
          {ProjectSettingsLive, :secrets}
        ] do
      assert {:ok, mounted} =
               view.mount(%{"project_id" => "project-1"}, %{}, disconnected_socket())

      assert %Phoenix.LiveView.LiveStream{} = mounted.assigns.streams[stream_name]
      assert Enum.empty?(mounted.assigns.streams[stream_name].inserts)
    end
  end

  test "ordinary browsing pages keep only their finite snapshot task" do
    for {view, params} <- [
          {OrganizationLive, %{}},
          {OrganizationWorkspaceLive, %{"organization_id" => "organization-1"}},
          {ProjectLive, %{"project_id" => "project-1"}},
          {OrganizationSecretsLive, %{"organization_id" => "organization-1"}},
          {ProjectSettingsLive, %{"project_id" => "project-1"}}
        ] do
      assert {:ok, mounted} = view.mount(params, %{}, disconnected_socket())
      refute Map.has_key?(mounted.assigns, :watch_task)
      assert mounted.assigns.snapshot_task == nil
    end
  end

  test "ordinary browsing pages do not create a watch after their snapshot completes" do
    organization = %{"id" => "organization-1", "name" => "Acme"}
    project = %{"id" => "project-1", "name" => "Forge", "organization_id" => "organization-1"}

    assert_snapshot_only(
      OrganizationLive,
      OrganizationState,
      %{},
      :load,
      {:loaded, 0, [organization]}
    )

    assert_snapshot_only(
      OrganizationWorkspaceLive,
      OrganizationWorkspaceState,
      %{"organization_id" => "organization-1"},
      :load,
      {:loaded, 1, organization, [project]}
    )

    assert_snapshot_only(
      ProjectLive,
      ProjectState,
      %{"project_id" => "project-1"},
      {:load, 1},
      {:loaded, 1, project, [%{"id" => "repository-1"}]}
    )

    assert_snapshot_only(
      OrganizationSecretsLive,
      OrganizationSecretsState,
      %{"organization_id" => "organization-1"},
      {:load, 0},
      {:loaded, 0, organization, [], []}
    )

    assert_snapshot_only(
      ProjectSettingsLive,
      ProjectSettingsState,
      %{"project_id" => "project-1"},
      {:load, 0},
      {:loaded, 0, project, [], %{"grants" => [], "imports" => []}, []}
    )
  end

  test "page-scoped task results transition every A-lane adapter to ready" do
    project = %{
      "id" => "project-1",
      "name" => "Forge",
      "organization_id" => "organization-1",
      "organization_name" => "Acme"
    }

    assert_ready(
      ProjectLive,
      ProjectState,
      %{"project_id" => "project-1"},
      {:loaded, 1, project, [%{"id" => "repository-1"}]}
    )

    assert_ready(
      ProjectAgentsLive,
      ProjectAgentsState,
      %{"project_id" => "project-1"},
      {:loaded, 1, project, [%{"id" => "instance-1"}], []}
    )

    assert_ready(
      ProjectRunsLive,
      ProjectRunsState,
      %{"project_id" => "project-1"},
      {:loaded, 1, project, [%{"id" => "run-1"}]}
    )

    assert_ready(
      ProjectSettingsLive,
      ProjectSettingsState,
      %{"project_id" => "project-1"},
      {:loaded, 1, project, [%{"id" => "secret-1"}], %{"grants" => [], "imports" => []}, []}
    )

    assert_ready(
      OrganizationSecretsLive,
      OrganizationSecretsState,
      %{"organization_id" => "organization-1"},
      {:loaded, 1, %{"id" => "organization-1", "name" => "Acme"}, [], []}
    )
  end

  test "an access-revoked snapshot schedules exactly one project navigation" do
    assert {:ok, mounted} =
             ProjectAgentsLive.mount(
               %{"project_id" => "project-1"},
               %{},
               disconnected_socket()
             )

    ref = make_ref()

    task = %Task{
      owner: self(),
      pid: self(),
      ref: ref,
      mfa: {__MODULE__, :access_revoked, 0}
    }

    socket = Phoenix.Component.assign(mounted, :snapshot_task, task)

    assert {:noreply, navigated} =
             ProjectAgentsLive.handle_info(
               {ref, {:access_revoked, Error.unavailable()}},
               socket
             )

    assert navigated.redirected == {:live, :redirect, %{kind: :push, to: "/organizations"}}
  end

  defp assert_ready(view, state_module, params, event) do
    assert {:ok, mounted} = view.mount(params, %{}, disconnected_socket())
    {loading, [:load]} = state_module.reduce(mounted.assigns.page_state, {:load, 1})
    ref = make_ref()

    task = %Task{
      owner: self(),
      pid: self(),
      ref: ref,
      mfa: {__MODULE__, :assert_ready, 4}
    }

    loading_socket =
      mounted
      |> Phoenix.Component.assign(:page_state, loading)
      |> Phoenix.Component.assign(:snapshot_task, task)

    assert {:noreply, ready_socket} =
             view.handle_info({ref, event}, loading_socket)

    assert ready_socket.assigns.page_state.status == :ready

    assert {:noreply, unchanged_socket} =
             view.handle_info({make_ref(), event}, ready_socket)

    assert unchanged_socket.assigns.page_state == ready_socket.assigns.page_state
  end

  defp assert_snapshot_only(view, state_module, params, load_event, loaded_event) do
    assert {:ok, mounted} = view.mount(params, %{}, disconnected_socket())
    {loading, [:load]} = state_module.reduce(mounted.assigns.page_state, load_event)
    ref = make_ref()

    task = %Task{
      owner: self(),
      pid: self(),
      ref: ref,
      mfa: {__MODULE__, :assert_snapshot_only, 5}
    }

    socket =
      mounted
      |> Phoenix.Component.assign(:page_state, loading)
      |> Phoenix.Component.assign(:snapshot_task, task)

    assert {:noreply, completed} = view.handle_info({ref, loaded_event}, socket)
    assert completed.assigns.page_state.status == :ready
    assert completed.assigns.snapshot_task == nil
    refute Map.has_key?(completed.assigns, :watch_task)
  end

  defp disconnected_socket do
    %Phoenix.LiveView.Socket{
      assigns: %{__changed__: %{}, current_identity: %{subject: "identity-1"}},
      private: %{live_temp: %{}, lifecycle: %Phoenix.LiveView.Lifecycle{}}
    }
  end
end
