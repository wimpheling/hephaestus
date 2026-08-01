defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryNewPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryNewPage

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

  test "renders every page state and the repository form" do
    for state <- @covered_states do
      html = render_component(&RepositoryNewPage.repository_new/1, assigns(state))

      if state == :ready do
        assert html =~ ~s(id="create-repository-form")
      else
        assert html =~ ~s(id="repository-new-page-state")
      end
    end

    html = render_component(&RepositoryNewPage.repository_new/1, assigns(:ready))
    assert html =~ ~s(id="create-repository-form")
    assert length(@covered_statuses) == 8
  end

  defp assigns(:ready) do
    %{
      state: :ready,
      project: %{
        "id" => "project-1",
        "name" => "Forge",
        "organization_id" => "organization-1",
        "organization_name" => "Acme"
      },
      form:
        Phoenix.Component.to_form(
          %{
            "name" => "",
            "default_branch" => "main",
            "is_public" => false,
            "agent_runs_enabled" => true
          },
          as: :repository
        ),
      create_event: "create-repository"
    }
  end

  defp assigns(state), do: Map.put(assigns(:ready), :state, state)
end
