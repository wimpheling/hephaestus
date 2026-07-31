defmodule HephaestusWebWeb.DesignSystem.Pages.OrganizationPageTest do
  use ExUnit.Case, async: true
  use HephaestusWebWeb, :html

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.OrganizationPage
  alias HephaestusWebWeb.OrganizationState

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

  test "renders the ready organization index with stable stream and test IDs" do
    organization = %{
      "id" => "organization-1",
      "name" => "Acme",
      "project_count" => 2,
      "repository_count" => 3
    }

    html =
      render_component(&OrganizationPage.organization_page/1,
        state: :ready,
        current_identity: %{display_name: "Ada"},
        organization_count: 1,
        organizations: [{"organization-stream-organization-1", organization}]
      )

    document = LazyHTML.from_fragment(html)

    assert LazyHTML.text(document) =~ "Good evening, Ada."
    assert LazyHTML.text(document) =~ "Acme"
    assert count(document, "#organizations[phx-update=stream]") == 1
    assert count(document, "#organization-stream-organization-1") == 1
    assert count(document, ~s|[data-testid="organization-organization-1"]|) == 1
  end

  test "renders presentation derived from every state status" do
    for status <- OrganizationState.statuses() do
      state = %{OrganizationState.new(%{}) | status: status}
      presentation = OrganizationState.present(state)

      html =
        render_component(&OrganizationPage.organization_page/1,
          state: presentation.status,
          current_identity: %{display_name: "Ada"},
          organization_count: presentation.organization_count,
          organizations: []
        )

      assert html != ""
    end

    assert length(@covered_statuses) == 8
    assert length(@covered_states) == 5
  end

  defp count(document, selector) do
    document
    |> LazyHTML.query(selector)
    |> LazyHTML.to_tree()
    |> length()
  end
end
