defmodule HephaestusWebWeb.DesignSystem.Pages.SessionChatNewPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.SessionChatNewPage
  alias HephaestusWebWeb.SessionChatNewState

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

  test "renders the release choice and explicit Git acknowledgement" do
    assert length(@covered_states) == 4
    assert @covered_statuses == SessionChatNewState.statuses()

    html =
      render_component(&SessionChatNewPage.session_chat_new/1,
        state: :ready,
        project: %{
          "organization_id" => "org-1",
          "organization_name" => "Acme",
          "name" => "Forge"
        },
        project_id: "project-1",
        release_catalog: [%{"id" => "release-1", "display_name" => "Reference session chat"}],
        form:
          Phoenix.Component.to_form(
            %{
              "repository_name" => "session-chat",
              "default_branch" => "main",
              "instance_name" => "session-chat",
              "release_agent_id" => "release-1",
              "model_import_id" => "import-1",
              "acknowledge_repository_git_access" => "false"
            },
            as: :session_chat
          ),
        create_event: "create-session-chat",
        secret_imports: [%{"id" => "import-1", "alias" => "model"}]
      )

    assert html =~ "Reference session chat"
    assert html =~ "acknowledge_repository_git_access"
    assert html =~ "Allow the release-owned UI to access this repository"
    assert html =~ ~s/pattern="[a-z0-9]([a-z0-9_]|-){0,127}"/
    assert html =~ "Use 1"
    refute html =~ "Model rule UUID"
    assert html =~ ~s(phx-hook="InstalledUiNavigation")
  end
end
