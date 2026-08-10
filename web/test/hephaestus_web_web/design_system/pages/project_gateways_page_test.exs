defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectGatewaysPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectGatewaysPage
  alias HephaestusWebWeb.ProjectGatewaysState

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

  test "renders the gateway collection for every lifecycle status" do
    assert @covered_statuses == ProjectGatewaysState.statuses()

    for status <- @covered_statuses do
      state = %{
        ProjectGatewaysState.new(%{project_id: "project-1"})
        | status: status,
          data: %{project_id: "project-1", gateways: [gateway()]}
      }

      presentation = ProjectGatewaysState.present(state)

      assert render_component(&ProjectGatewaysPage.project_gateways_page/1,
               state: presentation.status,
               project_id: "project-1",
               gateways: presentation.gateways,
               gateway_destination: &"/projects/project-1/gateways/#{&1}"
             ) != ""
    end

    assert length(@covered_states) == 5
  end

  defp gateway, do: %{"id" => "gateway-1", "name" => "Ingress", "lifecycle" => "enabled"}
end
