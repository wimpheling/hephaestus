defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectAgentsPageTest do
  use ExUnit.Case, async: true
  import Phoenix.LiveViewTest
  alias HephaestusWebWeb.DesignSystem.Pages.ProjectAgentsPage
  alias HephaestusWebWeb.ProjectAgentsState
  @covered_states [:loading, :empty, :error, :reconnecting, :ready]
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

  test "renders presentation derived from every state status" do
    assert @covered_statuses == ProjectAgentsState.statuses()

    for status <- @covered_statuses do
      state = ProjectAgentsState.new(%{project_id: "project-1"})

      state = %{
        state
        | status: status,
          data: %{
            state.data
            | project: project(),
              instances: [instance()],
              release_catalog: [release()]
          }
      }

      p = ProjectAgentsState.present(state)

      html =
        render_component(&ProjectAgentsPage.project_agents_page/1,
          state: p.status,
          project: p.project,
          project_id: p.project_id,
          item_count: p.item_count,
          instances: [{"project-instance-instance-1", instance()}],
          release_catalog: p.release_catalog,
          form: p.import_form,
          organization_index_destination: "/organizations",
          organization_destination: "/organizations/org-1",
          instance_destination: &"/projects/project-1/agents/#{&1}",
          import_event: "import-agent"
        )

      assert html != ""

      if status == :ready do
        document = LazyHTML.from_fragment(html)

        assert document
               |> LazyHTML.query(~s(select[name="import[parameters][review_style]"]))
               |> LazyHTML.to_tree()
               |> length() == 1

        assert document
               |> LazyHTML.query(~s(input[name="import[parameters][private_hint]"]))
               |> LazyHTML.to_tree()
               |> length() == 1

        assert html =~ "Import as new instance"
      end
    end

    assert length(@covered_states) == 5
  end

  defp project,
    do: %{
      "id" => "project-1",
      "name" => "Forge",
      "organization_id" => "org-1",
      "organization_name" => "Acme"
    }

  defp instance,
    do: %{
      "id" => "instance-1",
      "name" => "Cook",
      "release_version" => "1.0.0",
      "attachment_count" => 1,
      "run_count" => 2
    }

  defp release do
    %{
      "id" => "release-1",
      "display_name" => "Cook agent",
      "parameter_schema" => [
        %{
          "name" => "review_style",
          "value_type" => %{"type" => "enum", "values" => ["strict", "balanced"]},
          "required" => true,
          "default" => "balanced",
          "sensitive" => false
        },
        %{
          "name" => "private_hint",
          "value_type" => %{"type" => "string"},
          "required" => false,
          "default" => "",
          "sensitive" => true
        }
      ],
      "runtime_contract" => %{
        "policy_ceiling" => %{"vcpus" => 1, "memory_mib" => 128, "network" => "broker_only"}
      }
    }
  end
end
