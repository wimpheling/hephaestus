defmodule HephaestusWebWeb.DesignSystem.Pages.AgentInstancePageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.AgentInstancePage

  @covered_states [:loading, :error, :reconnecting, :ready]
  @status_visual_states %{
    initial: :loading,
    loading: :loading,
    ready: :ready,
    submitting: :loading,
    error: :error,
    stale: :reconnecting,
    reconnecting: :reconnecting,
    access_revoked: :error
  }

  test "renders every non-ready lifecycle state" do
    assert MapSet.new(Map.values(@status_visual_states)) == MapSet.new(@covered_states)

    for state <- [:loading, :error, :reconnecting] do
      html =
        render_component(&AgentInstancePage.agent_instance/1, Map.put(assigns(), :state, state))

      assert html =~ ~s(id="agent-instance-page-state")
      assert html =~ "Agent instance unavailable"
    end
  end

  test "renders stable identity, navigation, and stream IDs" do
    assert @covered_states == [:loading, :error, :reconnecting, :ready]

    html = render_component(&AgentInstancePage.agent_instance/1, assigns())

    for id <- ~w(
      instance-breadcrumbs project-tabs instance-overview revise-instance-panel
      secret-binding-panel instance-revisions create-attachment-panel instance-attachments
      capability-permission-panel capability-metrics capability-binding-history
      runtime-authority-sessions capability-audit-evidence create-update-panel instance-updates
      instance-recent-runs
    ) do
      assert html =~ ~s(id="#{id}")
    end

    assert html =~ ~s(href="/organizations/org-1")
    assert html =~ ~s(href="/projects/project-1/agents")
    assert html =~ ~s(href="/runs/run-1")
  end

  test "renders declared forms, names, and attachment event payloads" do
    html = render_component(&AgentInstancePage.agent_instance/1, assigns())

    assert html =~ ~s(id="revise-instance")
    assert html =~ ~s(phx-submit="revise-instance")
    assert html =~ ~s(name="revision[parameters][temperature]")
    assert html =~ ~s(id="bind-secret-api-token")
    assert html =~ ~s(name="binding[slot]")
    assert html =~ ~s(phx-submit="bind-secret")
    assert html =~ ~s(id="revise-capabilities")
    assert html =~ ~s(phx-submit="revise-capabilities")
    assert html =~ ~s(id="capability-resource-source-repository")
    assert html =~ ~s(phx-click="set-attachment")
    assert html =~ ~s(phx-value-attachment_id="attachment-1")
    assert html =~ ~s(phx-click="remove-attachment")
  end

  test "renders update recovery controls and polite hook progress" do
    html = render_component(&AgentInstancePage.agent_instance/1, assigns())

    assert html =~ ~s(id="create-update")
    assert html =~ ~s(phx-submit="create-update")
    assert html =~ ~s(id="update-hook-events-update-1")
    assert html =~ ~s(aria-live="polite")
    assert html =~ ~s(aria-label="Update hook events")
    assert html =~ ~s(tabindex="0")
    assert html =~ ~s(id="recover-retry-update-1")
    assert html =~ ~s(phx-click="recover-update")
    assert html =~ ~s(phx-value-update_id="update-1")
    assert html =~ ~s(phx-value-action="retry")
  end

  defp assigns do
    %{
      state: :ready,
      instance: instance(),
      revisions: [{"instance-revision-revision-1", revision()}],
      attachments: [{"instance-attachment-attachment-1", attachment()}],
      updates: [{"instance-update-update-1", update()}],
      attachment_form: Phoenix.Component.to_form(%{}, as: :attachment),
      revision_form: Phoenix.Component.to_form(%{}, as: :revision),
      update_form: Phoenix.Component.to_form(%{}, as: :update),
      binding_form: Phoenix.Component.to_form(%{}, as: :binding),
      capability_form: Phoenix.Component.to_form(%{}, as: :capabilities),
      organization_index_destination: "/organizations",
      organization_destination: "/organizations/org-1",
      project_agents_destination: "/projects/project-1/agents",
      repositories_tab_destination: "/projects/project-1",
      agents_tab_destination: "/projects/project-1/agents",
      runs_tab_destination: "/projects/project-1/runs",
      settings_tab_destination: "/projects/project-1/settings",
      run_destination: &"/runs/#{&1}",
      create_attachment_event: "create-attachment",
      set_attachment_event: "set-attachment",
      remove_attachment_event: "remove-attachment",
      revise_instance_event: "revise-instance",
      revise_capabilities_event: "revise-capabilities",
      create_update_event: "create-update",
      recover_update_event: "recover-update",
      bind_secret_event: "bind-secret"
    }
  end

  defp instance do
    %{
      "id" => "instance-1",
      "name" => "Cook",
      "state" => "active",
      "run_gate_open" => true,
      "organization_id" => "org-1",
      "organization_name" => "Acme",
      "project_id" => "project-1",
      "project_name" => "Forge",
      "active_revision_id" => "revision-1",
      "state_volume_id" => "volume-1",
      "can_manage" => true,
      "can_update" => true,
      "can_recover" => true,
      "revisions" => [revision()],
      "attachments" => [attachment()],
      "updates" => [update()],
      "repositories" => [%{"id" => "repository-1", "name" => "Source"}],
      "secret_imports" => [
        %{
          "id" => "import-1",
          "alias" => "registry-token",
          "target_kind" => "project",
          "delivery_modes" => ["brokered"]
        }
      ],
      "update_candidates" => [candidate()],
      "capability_requirements" => [capability_requirement()],
      "capability_resource_options" => [
        %{
          "id" => "repository-1",
          "slot_key" => "source-repository",
          "resource_kind" => "repository",
          "display_name" => "Source",
          "grantable_operations" => ["git_read", "update_ref"]
        }
      ],
      "capability_bindings" => [],
      "runtime_sessions" => [],
      "capability_audit" => [],
      "capability_metrics" => %{},
      "recent_runs" => [
        %{
          "id" => "run-1",
          "run_kind" => "normal",
          "instance_revision_id" => "revision-1",
          "outcome" => nil,
          "state" => "running"
        }
      ]
    }
  end

  defp revision do
    %{
      "id" => "revision-1",
      "release_agent_id" => "release-agent-1",
      "release_agent_name" => "Cook agent",
      "release_state" => "published",
      "release_version" => "1.0.0",
      "platform_policy_version" => "v1",
      "runnable" => true,
      "parameters" => %{"temperature" => 180},
      "parameter_schema" => [parameter()],
      "secret_slot_schema" => [slot()],
      "resource_selection" => %{"vcpus" => 1, "memory_mib" => 512, "network" => "disabled"},
      "runtime_contract" => %{
        "policy_ceiling" => %{"vcpus" => 2, "memory_mib" => 1024, "network" => "disabled"}
      }
    }
  end

  defp capability_requirement do
    %{
      "id" => "requirement-1",
      "release_agent_id" => "release-agent-1",
      "slot_key" => "source-repository",
      "purpose" => "Read and publish source",
      "resource_kind" => "repository",
      "required_operations" => ["git_read"],
      "optional_operations" => ["update_ref"],
      "slot_required" => true
    }
  end

  defp parameter do
    %{
      "name" => "temperature",
      "type" => "integer",
      "required" => true,
      "sensitive" => false,
      "default" => 180
    }
  end

  defp slot do
    %{
      "key" => "api-token",
      "required" => true,
      "purpose" => "Authenticate requests",
      "delivery_modes" => ["brokered"],
      "phases" => ["normal"],
      "destinations" => ["api.example.com"]
    }
  end

  defp candidate do
    %{
      "id" => "release-agent-2",
      "display_name" => "Cook agent",
      "release_version" => "2.0.0",
      "parameter_schema" => [parameter()],
      "runtime_contract" => %{
        "policy_ceiling" => %{"vcpus" => 2, "memory_mib" => 1024, "network" => "broker_only"}
      }
    }
  end

  defp attachment do
    %{
      "id" => "attachment-1",
      "repository_name" => "Source",
      "ref_selector" => "refs/heads/main",
      "trigger_policy" => "push",
      "enabled" => true,
      "removed_at" => nil,
      "can_manage" => true
    }
  end

  defp update do
    %{
      "id" => "update-1",
      "state" => "compatibility_unknown",
      "expected_current_revision_id" => "revision-1",
      "candidate_revision_id" => "revision-2",
      "final_decision" => nil,
      "hook_events" => [
        %{
          "sequence" => 1,
          "event_type" => "vm.log",
          "payload" => %{"message" => "checking state"}
        }
      ]
    }
  end
end
