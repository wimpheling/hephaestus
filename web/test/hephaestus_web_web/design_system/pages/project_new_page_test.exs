defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectNewPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectNewPage

  @covered_states [:loading, :error, :reconnecting, :ready]
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

  test "renders every page state and the project form" do
    for state <- @covered_states do
      html = render_component(&ProjectNewPage.project_new/1, assigns(state))

      if state == :ready do
        assert html =~ ~s(id="create-project-form")
      else
        assert html =~ ~s(id="project-new-page-state")
      end
    end

    html = render_component(&ProjectNewPage.project_new/1, assigns(:ready))
    assert html =~ ~s(id="create-project-form")
    assert html =~ "Description"
    assert length(@covered_statuses) == 8
  end

  defp assigns(:ready) do
    %{
      state: :ready,
      organization: %{"id" => "organization-1", "name" => "Acme"},
      form: Phoenix.Component.to_form(%{"name" => "", "description" => ""}, as: :project),
      create_event: "create-project"
    }
  end

  defp assigns(state), do: Map.put(assigns(:ready), :state, state)
end
