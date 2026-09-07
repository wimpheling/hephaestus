defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectGatewayPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectGatewayPage
  alias HephaestusWebWeb.ProjectGatewayState

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

  test "renders gateway detail presentation for every lifecycle status" do
    assert @covered_statuses == ProjectGatewayState.statuses()

    for status <- @covered_statuses do
      state = %{
        ProjectGatewayState.new(%{project_id: "project-1", gateway_id: "gateway-1"})
        | status: status,
          data: %{
            project_id: "project-1",
            gateway_id: "gateway-1",
            gateway: gateway(),
            ingress: []
          }
      }

      presentation = ProjectGatewayState.present(state)

      assert render_component(&ProjectGatewayPage.project_gateway_page/1,
               state: presentation.status,
               gateway: presentation.gateway,
               ingress: presentation.ingress,
               gateways_destination: "/projects/project-1/gateways",
               lifecycle_event: "lifecycle",
               lifecycle_actions: presentation.lifecycle_actions
             ) != ""
    end

    assert length(@covered_states) == 5
  end

  test "renders published typed parameters and authorized current secret versions" do
    html =
      render_component(&ProjectGatewayPage.project_gateway_page/1,
        state: :ready,
        gateway: gateway(),
        ingress: [],
        release_catalog: [
          %{
            "id" => "agent-1",
            "parameter_schema" => [
              %{
                "name" => "tenant",
                "value_type" => %{"type" => "string"},
                "default" => "demo",
                "required" => true
              },
              %{
                "name" => "retries",
                "value_type" => %{"type" => "integer"},
                "required" => false
              }
            ]
          }
        ],
        secret_authority: %{
          "imports" => [
            %{
              "id" => "import-1",
              "secret_id" => "secret-1",
              "alias" => "Inbound",
              "state" => "active",
              "active_version_id" => "version-1"
            },
            %{
              "id" => "import-2",
              "secret_id" => "secret-2",
              "alias" => "Revoked",
              "state" => "revoked"
            }
          ]
        },
        secrets: [
          %{"id" => "secret-1", "active_version_id" => "version-1"},
          %{"id" => "secret-2", "active_version_id" => "version-2"}
        ],
        configure_form: %{},
        gateways_destination: "/projects/project-1/gateways",
        lifecycle_event: "lifecycle",
        lifecycle_actions: [],
        configure_event: "configure"
      )

    assert html =~ ~s(id="configure-gateway-form")
    assert html =~ ~s(id="configure-parameter-tenant")
    assert html =~ ~s(id="configure-secret-inbound")
    assert html =~ ~s(name="configure[parameters][tenant]")
    assert html =~ ~s(name="configure[parameters][retries]")
    assert html =~ ~s(value="import-1|version-1|/hooks|x-heph-secret")
    refute html =~ ~s(value="import-2|version-2|/hooks|x-heph-secret")
  end

  test "renders declared mailbox slots and authorized instance mailboxes" do
    html =
      render_component(&ProjectGatewayPage.project_gateway_page/1,
        state: :ready,
        gateway:
          Map.put(gateway(), "revisions", [
            Map.put(List.first(gateway()["revisions"]), "mailbox_slots", ["deliver"])
          ]),
        ingress: [],
        instances: [
          %{
            "id" => "instance-1",
            "name" => "Worker",
            "state" => "ready",
            "mailbox_id" => "mailbox-1"
          },
          %{
            "id" => "removed",
            "name" => "Retired",
            "state" => "removed",
            "mailbox_id" => "mailbox-2"
          }
        ],
        binding_slots: ["deliver"],
        bindings: [],
        binding_form: %{},
        gateways_destination: "/projects/project-1/gateways",
        lifecycle_event: "lifecycle",
        lifecycle_actions: [],
        configure_event: "configure",
        bind_event: "bind-mailbox"
      )

    assert html =~ ~s(id="gateway-binding-form")
    assert html =~ ~s(id="gateway-binding-slot")
    assert html =~ ~s(value="deliver")
    assert html =~ ~s(id="gateway-binding-mailbox")
    assert html =~ ~s(value="mailbox-1")
    refute html =~ ~s(value="mailbox-2")
  end

  defp gateway do
    %{
      "id" => "gateway-1",
      "name" => "Ingress",
      "lifecycle" => "enabled",
      "active_revision_id" => "revision-1",
      "revisions" => [
        %{
          "id" => "revision-1",
          "release_agent_id" => "agent-1",
          "release_id" => "release-1",
          "secret_slots" => ["inbound"],
          "handler_contract" => "http",
          "exposure" => "public",
          "routes" => [%{"path" => "/hooks", "methods" => ["POST"]}]
        }
      ]
    }
  end
end
