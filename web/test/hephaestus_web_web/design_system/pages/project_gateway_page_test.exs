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

  defp gateway do
    %{
      "id" => "gateway-1",
      "name" => "Ingress",
      "lifecycle" => "enabled",
      "revisions" => [
        %{
          "id" => "revision-1",
          "handler_contract" => "http",
          "exposure" => "public",
          "routes" => []
        }
      ]
    }
  end
end
