defmodule HephaestusWebWeb.StateReducerTransitionTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.AgentInstanceState
  alias HephaestusWebWeb.OrganizationNewGrantState
  alias HephaestusWebWeb.OrganizationNewSecretState
  alias HephaestusWebWeb.OrganizationSecretsState
  alias HephaestusWebWeb.OrganizationState
  alias HephaestusWebWeb.OrganizationWorkspaceState
  alias HephaestusWebWeb.ProjectAgentsState
  alias HephaestusWebWeb.ProjectRunsState
  alias HephaestusWebWeb.ProjectSettingsState
  alias HephaestusWebWeb.ProjectState
  alias HephaestusWebWeb.ReleaseState
  alias HephaestusWebWeb.RepositoryAgentsState
  alias HephaestusWebWeb.RepositoryBranchesState
  alias HephaestusWebWeb.RepositoryCommitsState
  alias HephaestusWebWeb.RepositoryFilesState
  alias HephaestusWebWeb.RepositoryReleasesState
  alias HephaestusWebWeb.RunState

  @agent_events [
    "create-attachment",
    "set-attachment",
    "remove-attachment",
    "revise-instance",
    "create-update",
    "recover-update",
    "bind-secret"
  ]
  @run_controls ["cancel_run", "retry_run", "approve_result", "reject_result"]

  test "covers every organization index and workspace reducer transition" do
    organization = OrganizationState.new(%{})
    assert {loading, [:load]} = OrganizationState.reduce(organization, :load)
    assert loading.status == :loading
    assert {ready, []} = OrganizationState.reduce(loading, {:loaded, [%{"id" => "org-1"}]})
    assert ready.status == :ready
    assert {failed, []} = OrganizationState.reduce(ready, {:failed, :unavailable})
    assert failed.status == :error
    assert {reconnecting, []} = OrganizationState.reduce(ready, :reconnecting)
    assert reconnecting.status == :reconnecting
    assert {stale, [:load]} = OrganizationState.reduce(ready, :stale)
    assert stale.status == :stale

    workspace = OrganizationWorkspaceState.new(%{organization_id: "org-1"})
    assert {loading, [:load]} = OrganizationWorkspaceState.reduce(workspace, :load)

    assert {ready, []} =
             OrganizationWorkspaceState.reduce(
               loading,
               {:loaded, %{"id" => "org-1"}, [%{"id" => "project-1"}]}
             )

    assert ready.status == :ready

    assert {revoked, [{:navigate, :organizations}]} =
             OrganizationWorkspaceState.reduce(ready, {:access_revoked, :forbidden})

    assert revoked.status == :access_revoked
    assert {reconnecting, []} = OrganizationWorkspaceState.reduce(ready, :reconnecting)
    assert reconnecting.status == :reconnecting
    assert {stale, [:load]} = OrganizationWorkspaceState.reduce(ready, :stale)
    assert stale.status == :stale
  end

  test "covers every organization secret and grant reducer transition" do
    grant = OrganizationNewGrantState.new(%{organization_id: "org-1"})
    assert {loading, [:load]} = OrganizationNewGrantState.reduce(grant, :load)
    assert {submitting, []} = OrganizationNewGrantState.reduce(loading, :submitting)
    assert submitting.status == :submitting

    assert {ready, []} =
             OrganizationNewGrantState.reduce(
               loading,
               {:loaded, %{"id" => "org-1"}, [], [], []}
             )

    assert ready.status == :ready
    assert_error_transitions(OrganizationNewGrantState, ready)

    secret = OrganizationNewSecretState.new(%{organization_id: "org-1"})
    assert {loading, [:load]} = OrganizationNewSecretState.reduce(secret, :load)
    assert {submitting, []} = OrganizationNewSecretState.reduce(loading, :submitting)
    assert submitting.status == :submitting

    assert {ready, []} =
             OrganizationNewSecretState.reduce(
               loading,
               {:loaded, %{"id" => "org-1"}, ["existing"]}
             )

    assert ready.status == :ready
    assert_error_transitions(OrganizationNewSecretState, ready)

    secrets = OrganizationSecretsState.new(%{organization_id: "org-1"})
    assert {loading, [:load]} = OrganizationSecretsState.reduce(secrets, {:load, 4})

    assert {ready, []} =
             OrganizationSecretsState.reduce(
               loading,
               {:loaded, 4, %{"id" => "org-1"}, [%{"id" => "secret-1"}], []}
             )

    assert ready.status == :ready
    assert OrganizationSecretsState.reduce(ready, {:loaded, 3, %{}, [], []}) == {ready, []}
    assert {submitting, []} = OrganizationSecretsState.reduce(ready, :submitting)
    assert submitting.status == :submitting
    assert_error_transitions(OrganizationSecretsState, ready)
  end

  test "covers every project reducer transition" do
    project = ProjectState.new(%{project_id: "project-1"})
    project = assert_generation_reducer(ProjectState, project, [%{"id" => "repository-1"}])
    assert_project_terminal_transitions(ProjectState, project)

    runs = ProjectRunsState.new(%{project_id: "project-1"})
    runs = assert_generation_reducer(ProjectRunsState, runs, [%{"id" => "run-1"}])
    assert_project_terminal_transitions(ProjectRunsState, runs)

    agents = ProjectAgentsState.new(%{project_id: "project-1"})
    assert {loading, [:load]} = ProjectAgentsState.reduce(agents, {:load, 7})

    assert {ready, []} =
             ProjectAgentsState.reduce(
               loading,
               {:loaded, 7, %{"id" => "project-1"}, [%{"id" => "agent-1"}], []}
             )

    assert ProjectAgentsState.reduce(ready, {:loaded, 6, %{}, [], []}) == {ready, []}
    assert {submitting, []} = ProjectAgentsState.reduce(ready, :submitting)
    assert submitting.status == :submitting
    assert_error_transitions(ProjectAgentsState, ready)

    settings = ProjectSettingsState.new(%{project_id: "project-1"})
    assert {loading, [:load]} = ProjectSettingsState.reduce(settings, {:load, 9})

    assert {ready, []} =
             ProjectSettingsState.reduce(
               loading,
               {:loaded, 9, %{"id" => "project-1"}, [], %{}, []}
             )

    assert ProjectSettingsState.reduce(ready, {:loaded, 8, %{}, [], %{}, []}) == {ready, []}
    assert {submitting, []} = ProjectSettingsState.reduce(ready, :submitting)
    assert submitting.status == :submitting
    assert_error_transitions(ProjectSettingsState, ready)
  end

  test "covers every shared repository route reducer transition" do
    routes = [
      {RepositoryAgentsState, :agents},
      {RepositoryBranchesState, :branches},
      {RepositoryCommitsState, :commits},
      {RepositoryFilesState, :files},
      {RepositoryReleasesState, :releases}
    ]

    for {module, action} <- routes do
      state = module.new("repository-1")
      uri = "/repositories/repository-1/#{action}"

      assert {loading, [{:load, 1, ^action, "repository-1", %{}, ^uri}]} =
               module.reduce(state, {:load, %{}, uri})

      assert loading.status == :loading
      assert {still_loading, []} = module.reduce(loading, :connected)
      assert still_loading.status == :loading
      assert {reconnecting, []} = module.reduce(loading, :disconnected)
      assert reconnecting.status == :reconnecting

      data = %{
        loading.data
        | repository: %{"id" => "repository-1"},
          selected_branch: %{name: "main"},
          params: %{},
          uri: uri
      }

      assert {ready, []} = module.reduce(loading, {:loaded, 1, {:ok, data}})
      assert ready.status == :ready
      assert ready.cursor == nil
      assert module.reduce(ready, {:loaded, 0, {:ok, data}}) == {ready, []}

      assert {refreshing, [{:load, 1, ^action, "repository-1", %{}, ^uri}]} =
               module.reduce(ready, :refresh)

      assert refreshing.status == :stale

      assert {connected, [{:load, 1, ^action, "repository-1", %{}, ^uri}]} =
               module.reduce(ready, :connected)

      assert connected.status == :stale

      assert {revoked, effects} = module.reduce(loading, {:loaded, 1, {:error, :forbidden}})
      assert revoked.status == :access_revoked
      assert {:navigate, "/organizations"} in effects

      assert {failed, [{:flash, :error, _message}]} =
               module.reduce(ready, {:effect_failed, :down})

      assert failed.status == :error
    end

    assert {_, [{:patch, "/repositories/repository-1/commits?ref=feature%2Fone"}]} =
             RepositoryCommitsState.new("repository-1")
             |> RepositoryCommitsState.reduce({:select_branch, "feature/one"})

    assert {_, [{:patch, "/repositories/repository-1/files?ref=feature%2Fone"}]} =
             RepositoryFilesState.new("repository-1")
             |> RepositoryFilesState.reduce({:select_branch, "feature/one"})
  end

  test "covers every agent lifecycle, interaction, and command-result transition" do
    state = AgentInstanceState.new("instance-1")
    assert {loading, [{:load, 1, "instance-1"}]} = AgentInstanceState.reduce(state, :load)
    assert {refreshing, [{:load, 1, "instance-1"}]} = AgentInstanceState.reduce(state, :refresh)
    assert refreshing.status == :stale
    assert {reconnecting, []} = AgentInstanceState.reduce(loading, :disconnected)
    assert reconnecting.status == :reconnecting

    assert {connected_loading, [{:load, 2, "instance-1"}]} =
             AgentInstanceState.reduce(loading, :connected)

    assert connected_loading.status == :loading

    assert {ready, []} =
             AgentInstanceState.reduce(loading, {:loaded, 1, {:ok, agent_instance()}})

    assert {connected_ready, []} = AgentInstanceState.reduce(ready, :connected)
    assert connected_ready.status == :ready

    for event <- @agent_events do
      assert {submitting, [{:command, 1, ^event, params}]} =
               AgentInstanceState.reduce(ready, {:interaction, event, %{}})

      assert submitting.status == :submitting
      assert params["instance_id"] == "instance-1"
    end

    assert {pending, []} =
             ready
             |> Map.put(:status, :submitting)
             |> AgentInstanceState.reduce(:refresh)

    assert pending.data.refresh_pending

    assert {revoked, [{:navigate, "/organizations"}]} =
             AgentInstanceState.reduce(loading, {:loaded, 1, {:error, :forbidden}})

    assert revoked.status == :access_revoked

    assert AgentInstanceState.reduce(loading, {:loaded, 0, {:ok, agent_instance()}}) ==
             {loading, []}

    receipt = mutation_receipt()

    assert {completed, [{:flash, :info, "Updated"}]} =
             AgentInstanceState.reduce(
               ready,
               {:command_completed, 1, {:ok, receipt, "Updated"}}
             )

    assert completed.status == :stale

    assert {refresh_after_command, effects} =
             AgentInstanceState.reduce(
               %{pending | stream_generation: 1},
               {:command_completed, 1, {:ok, receipt, "Updated"}}
             )

    assert refresh_after_command.status == :stale
    assert [{:flash, :info, "Updated"}] == effects

    assert {failed, [{:flash, :error, "Denied"}]} =
             AgentInstanceState.reduce(ready, {:command_completed, 1, {:error, "Denied"}})

    assert failed.status == :error

    assert {revoked, effects} =
             AgentInstanceState.reduce(ready, {:command_completed, 1, :access_revoked})

    assert revoked.status == :access_revoked
    assert {:navigate, "/organizations"} in effects

    assert AgentInstanceState.reduce(ready, {:command_completed, 0, {:error, "stale"}}) ==
             {ready, []}

    assert {failed, [{:flash, :error, _message}]} =
             AgentInstanceState.reduce(ready, {:effect_failed, :down})

    assert failed.status == :error
  end

  test "covers every run lifecycle, control, and command-result transition" do
    state = RunState.new("run-1")
    assert {loading, [{:load, 1, "run-1"}]} = RunState.reduce(state, :load)
    assert {refreshing, [{:load, 1, "run-1"}]} = RunState.reduce(state, :refresh)
    assert refreshing.status == :stale
    assert {reconnecting, []} = RunState.reduce(loading, :disconnected)
    assert reconnecting.status == :reconnecting
    assert {connected_loading, [{:load, 2, "run-1"}]} = RunState.reduce(loading, :connected)
    assert connected_loading.status == :loading

    assert {ready, []} = RunState.reduce(loading, {:loaded, 1, {:ok, run()}})
    assert {connected_ready, [{:load, 2, "run-1"}]} = RunState.reduce(ready, :connected)
    assert connected_ready.status == :stale

    for kind <- @run_controls do
      assert {submitting, [{:control, 1, payload}]} =
               RunState.reduce(ready, {:control, %{"kind" => kind, "reason" => "because"}})

      assert submitting.status == :submitting
      assert payload["kind"] == kind
      assert payload["run_lookup_id"] == "run-1"
    end

    assert {revoked, effects} = RunState.reduce(loading, {:loaded, 1, {:error, :forbidden}})
    assert revoked.status == :access_revoked
    assert {:navigate, "/organizations"} in effects
    assert RunState.reduce(loading, {:loaded, 0, {:ok, run()}}) == {loading, []}

    assert {completed, [{:flash, :info, "Approved"}]} =
             RunState.reduce(
               ready,
               {:control_completed, 1, {:ok, mutation_receipt(), "Approved"}}
             )

    assert completed.status == :stale

    assert {failed, [{:flash, :error, _message}]} =
             RunState.reduce(ready, {:control_completed, 1, {:error, :denied}})

    assert failed.status == :error
    assert RunState.reduce(ready, {:control_completed, 0, {:error, :stale}}) == {ready, []}

    assert {failed, [{:flash, :error, _message}]} =
             RunState.reduce(ready, {:effect_failed, :down})

    assert failed.status == :error
  end

  test "covers every release lifecycle and effect-result transition" do
    state = ReleaseState.new("release-1")
    assert {loading, [{:load, 1, "release-1"}]} = ReleaseState.reduce(state, :load)
    assert {reconnecting, []} = ReleaseState.reduce(loading, :disconnected)
    assert reconnecting.status == :reconnecting
    assert {connected, [{:load, 2, "release-1"}]} = ReleaseState.reduce(loading, :connected)
    assert connected.status == :loading

    assert {ready, []} = ReleaseState.reduce(loading, {:loaded, 1, {:ok, release()}})
    assert ready.status == :ready
    assert ReleaseState.reduce(loading, {:loaded, 0, {:ok, release()}}) == {loading, []}

    assert {revoked, [{:navigate, "/organizations"}]} =
             ReleaseState.reduce(loading, {:loaded, 1, {:error, :forbidden}})

    assert revoked.status == :access_revoked
    assert {failed, []} = ReleaseState.reduce(ready, {:effect_failed, :down})
    assert failed.status == :error
  end

  defp assert_error_transitions(module, state) do
    assert {failed, [{:flash, _message}]} = module.reduce(state, {:failed, :unavailable})
    assert failed.status == :error

    assert {revoked, [{:navigate, :organizations}]} =
             module.reduce(state, {:access_revoked, :forbidden})

    assert revoked.status == :access_revoked
    assert {reconnecting, []} = module.reduce(state, :reconnecting)
    assert reconnecting.status == :reconnecting
    assert {stale, [:load]} = module.reduce(state, :stale)
    assert stale.status == :stale
  end

  defp assert_generation_reducer(module, state, items) do
    assert {loading, [:load]} = module.reduce(state, {:load, 3})
    loaded_event = generation_loaded_event(module, 3, items)
    stale_event = generation_loaded_event(module, 2, [])
    assert {ready, []} = module.reduce(loading, loaded_event)
    assert module.reduce(ready, stale_event) == {ready, []}
    ready
  end

  defp generation_loaded_event(ProjectState, generation, repositories),
    do: {:loaded, generation, %{"id" => "project-1"}, repositories}

  defp generation_loaded_event(ProjectRunsState, generation, runs),
    do: {:loaded, generation, %{"id" => "project-1"}, runs}

  defp assert_project_terminal_transitions(module, state) do
    assert {revoked, [{:navigate, :organizations}]} =
             module.reduce(state, {:access_revoked, :forbidden})

    assert revoked.status == :access_revoked
    assert {reconnecting, []} = module.reduce(state, :reconnecting)
    assert reconnecting.status == :reconnecting
    assert {stale, [:load]} = module.reduce(state, :stale)
    assert stale.status == :stale
  end

  defp agent_instance do
    %{
      "id" => "instance-1",
      "organization_id" => "org-1",
      "project_id" => "project-1",
      "active_revision_id" => "revision-1",
      "revisions" => [],
      "attachments" => [],
      "updates" => []
    }
  end

  defp run do
    %{
      "id" => "run-1",
      "organization_id" => "org-1",
      "repository_id" => "repository-1",
      "source_repository_id" => "repository-1",
      "release_id" => "release-1",
      "instance_project_id" => "project-1",
      "agent_id" => "agent-1",
      "proposal_id" => "proposal-1",
      "events" => [],
      "artifacts" => [],
      "patch_preview" => nil,
      "manifest_preview" => nil
    }
  end

  defp release do
    %{
      "id" => "release-1",
      "organization_id" => "org-1",
      "project_id" => "project-1",
      "repository_id" => "repository-1",
      "source_ref" => "refs/heads/main",
      "artifacts" => [],
      "agents" => []
    }
  end

  defp mutation_receipt do
    %{committed_cursor: "cursor-2", event_id: "event-2", aggregate_version: 2}
  end
end
