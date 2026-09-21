defmodule HephaestusWebWeb.SessionChatNewStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.SessionChatNewState

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

  test "keeps the Git acknowledgement in the transient form and exposes the release catalog" do
    assert @covered_statuses == SessionChatNewState.statuses()
    state = SessionChatNewState.new("project-1")
    assert :initial == state.status
    assert state.form["acknowledge_repository_git_access"] == "false"

    {ready, []} =
      SessionChatNewState.reduce(
        state,
        {:loaded,
         {:ok, %{"id" => "project-1"}, [%{"id" => "release-1"}], [%{"id" => "import-1"}]}}
      )

    assert ready.status == :ready
    assert ready.data.release_catalog == [%{"id" => "release-1"}]
    assert ready.data.secret_imports == [%{"id" => "import-1"}]
  end

  test "failed composition reports a visible error and never launches a browser" do
    state = SessionChatNewState.new("project-1")

    {failed, effects} =
      SessionChatNewState.reduce(state, {:created, {:error, :release_unavailable}})

    assert failed.status == :error
    assert failed.error == "The selected release is no longer available."
    assert effects == [{:flash, :error, "The selected release is no longer available."}]
  end

  test "retains IDs created before a later composition step fails" do
    state = SessionChatNewState.new("project-1")
    progress = %{"repository_id" => "repo-1", "instance_id" => "instance-1"}

    {failed, effects} =
      SessionChatNewState.reduce(state, {:created, {:partial, progress, :install_failed}})

    assert failed.status == :error
    assert failed.data.progress == progress

    assert effects == [
             {:flash, :error, "Session setup stopped after creating instance_id, repository_id."}
           ]
  end

  test "resumes a failed capability step without creating another repository or instance" do
    state = SessionChatNewState.new("project-1")

    state =
      %{
        state
        | data:
            Map.merge(state.data, %{
              __client: HephaestusWebWeb.SessionChatNewStateTest.FakeClient,
              __attempt_id: "00000000-0000-4000-8000-000000000001"
            })
      }

    state = %{
      state
      | data: %{
          state.data
          | project: %{"organization_id" => "org-1"},
            release_catalog: [release_agent()],
            secret_imports: [model_import()]
        }
    }

    attributes = %{
      "repository_name" => "session-chat",
      "default_branch" => "main",
      "instance_name" => "Session chat",
      "release_agent_id" => "release-agent-1",
      "model_rule_id" => "00000000-0000-4000-8000-000000000007",
      "model_import_id" => "import-1",
      "acknowledge_repository_git_access" => "true",
      "setup_attempt_id" => "00000000-0000-4000-8000-000000000001"
    }

    assert {:created, {:partial, progress, :capability_unavailable}} =
             SessionChatNewState.execute(
               state,
               {:create, :identity, attributes}
             )

    {retry_state, _effects} =
      SessionChatNewState.reduce(
        state,
        {:created, {:partial, progress, :capability_unavailable}}
      )

    assert {:created, {:ok, "https://ui.test/session-chat", "repo-1", "instance-1"}} =
             SessionChatNewState.execute(retry_state, {:create, :identity, attributes})

    assert Process.get(:session_chat_fake_calls) == %{
             create_repository: 1,
             import_agent: 1,
             revise_capabilities: 2,
             create_attachment: 1,
             bind_secret: 1,
             install_ui: 1,
             handoff: 1
           }
  end

  defp release_agent do
    %{
      "id" => "release-agent-1",
      "release_id" => "release-1",
      "parameter_schema" => [%{"name" => "model_rule_id", "required" => true}],
      "secret_slot_schema" => [
        %{
          "key" => "model",
          "required" => true,
          "delivery_modes" => ["brokered"],
          "phases" => ["normal"],
          "destinations" => ["api.model.example"]
        }
      ],
      "capability_requirements" => [
        %{
          "slot_key" => "session",
          "resource_kind" => "repository",
          "required_operations" => ["git_read", "update_ref"]
        }
      ],
      "runtime_contract" => %{"policy_ceiling" => %{"vcpus" => 1, "memory_mib" => 256}}
    }
  end

  defp model_import do
    %{
      "id" => "import-1",
      "policy" => %{
        "delivery_modes" => ["brokered"],
        "phases" => ["normal"],
        "destinations" => ["api.model.example"]
      }
    }
  end
end
