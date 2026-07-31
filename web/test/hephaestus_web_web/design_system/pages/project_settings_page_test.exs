defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectSettingsPageTest do
  use ExUnit.Case, async: true
  import Phoenix.LiveViewTest
  alias HephaestusWebWeb.DesignSystem.Pages.ProjectSettingsPage
  alias HephaestusWebWeb.ProjectSettingsState
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

  test "renders presentation derived from every state status and declared secret events" do
    assert @covered_statuses == ProjectSettingsState.statuses()

    for status <- @covered_statuses do
      state = ProjectSettingsState.new(%{project_id: "project-1"})

      state = %{
        state
        | status: status,
          data: %{state.data | project: project(), secrets: [secret()]}
      }

      p = ProjectSettingsState.present(state)

      forms = %{
        secret: p.secret_form,
        grant: p.grant_form,
        import: p.import_form,
        rotate: p.rotate_form
      }

      html =
        render_component(&ProjectSettingsPage.project_settings_page/1,
          state: p.status,
          project: p.project,
          project_id: p.project_id,
          item_count: p.item_count,
          secrets: [{"project-secret-secret-1", secret()}],
          project_secrets: p.secrets,
          secret_authority: p.secret_authority,
          project_repositories: p.repositories,
          form: forms,
          organization_index_destination: "/organizations",
          organization_destination: "/organizations/org-1",
          create_secret_event: "create-secret",
          rotate_secret_event: "rotate-secret",
          set_secret_enabled_event: "set-secret-enabled",
          revoke_secret_event: "revoke-secret",
          purge_secret_event: "purge-secret",
          grant_secret_event: "grant-secret",
          accept_import_event: "accept-secret-import"
        )

      assert html != ""

      if status == :ready do
        document = LazyHTML.from_fragment(html)

        for name <- ["secret[modes][]", "grant[modes][]", "grant[phases][]"] do
          assert document
                 |> LazyHTML.query(~s(select[multiple][name="#{name}"]))
                 |> LazyHTML.to_tree()
                 |> length() == 1
        end
      end
    end

    assert length(@covered_states) == 5
  end

  test "renders one event-bound acceptance form for each pending grant" do
    state = ProjectSettingsState.new(%{project_id: "project-1"})

    authority = %{
      "grants" => [
        %{
          "id" => "grant-1",
          "import_id" => nil,
          "secret_name" => "organization_token",
          "target_kind" => "project",
          "target_id" => "project-1",
          "delivery_modes" => ["raw"]
        }
      ],
      "imports" => []
    }

    {loading, [:load]} = ProjectSettingsState.reduce(state, {:load, 1})

    {ready, []} =
      ProjectSettingsState.reduce(
        loading,
        {:loaded, 1, project(), [], authority, []}
      )

    presentation = ProjectSettingsState.present(ready)

    html =
      render_component(&ProjectSettingsPage.project_settings_page/1,
        state: presentation.status,
        project: presentation.project,
        project_id: presentation.project_id,
        item_count: presentation.item_count,
        secrets: [],
        project_secrets: presentation.secrets,
        secret_authority: presentation.secret_authority,
        project_repositories: presentation.repositories,
        form: %{
          secret: presentation.secret_form,
          grant: presentation.grant_form,
          import: presentation.import_form,
          rotate: presentation.rotate_form
        },
        organization_index_destination: "/organizations",
        organization_destination: "/organizations/org-1",
        create_secret_event: "create-secret",
        rotate_secret_event: "rotate-secret",
        set_secret_enabled_event: "set-secret-enabled",
        revoke_secret_event: "revoke-secret",
        purge_secret_event: "purge-secret",
        grant_secret_event: "grant-secret",
        accept_import_event: "accept-secret-import"
      )

    document = LazyHTML.from_fragment(html)
    form = LazyHTML.query(document, "#accept-import-grant-1")

    assert form
           |> LazyHTML.query(~s(input[name="secret_import[grant_id]"][value="grant-1"]))
           |> LazyHTML.to_tree()
           |> length() == 1

    assert form
           |> LazyHTML.query(~s(input[name="secret_import[alias]"]))
           |> LazyHTML.to_tree()
           |> length() == 1

    assert html =~ ~s(phx-submit="accept-secret-import")
    assert LazyHTML.text(form) =~ "Accept live reference"
  end

  test "renders populated project secret creation and grant controls" do
    state = ProjectSettingsState.new(%{project_id: "project-1"})
    secret = Map.put(secret(), "name", "project_token")
    repository = %{"id" => "repository-1", "name" => "agent-workbench"}
    {loading, [:load]} = ProjectSettingsState.reduce(state, {:load, 1})

    {ready, []} =
      ProjectSettingsState.reduce(
        loading,
        {:loaded, 1, project(), [secret], %{"grants" => [], "imports" => []}, [repository]}
      )

    presentation = ProjectSettingsState.present(ready)

    html =
      render_component(&ProjectSettingsPage.project_settings_page/1,
        state: presentation.status,
        project: presentation.project,
        project_id: presentation.project_id,
        item_count: presentation.item_count,
        secrets: [{"project-secret-secret-1", secret}],
        project_secrets: presentation.secrets,
        secret_authority: presentation.secret_authority,
        project_repositories: presentation.repositories,
        form: %{
          secret: presentation.secret_form,
          grant: presentation.grant_form,
          import: presentation.import_form,
          rotate: presentation.rotate_form
        },
        organization_index_destination: "/organizations",
        organization_destination: "/organizations/org-1",
        create_secret_event: "create-secret",
        rotate_secret_event: "rotate-secret",
        set_secret_enabled_event: "set-secret-enabled",
        revoke_secret_event: "revoke-secret",
        purge_secret_event: "purge-secret",
        grant_secret_event: "grant-secret",
        accept_import_event: "accept-secret-import"
      )

    document = LazyHTML.from_fragment(html)
    create = LazyHTML.query(document, "#create-project-secret")
    grant = LazyHTML.query(document, "#grant-secret")

    assert LazyHTML.text(create) =~ "Encrypt and create"
    assert html =~ ~s(id="project-secret-stream")

    assert grant
           |> LazyHTML.query(~s(select[name="grant[secret_id]"] option[value="secret-1"]))
           |> LazyHTML.to_tree()
           |> length() == 1

    assert grant
           |> LazyHTML.query(
             ~s(select[name="grant[target]"] option[value="repository:repository-1"])
           )
           |> LazyHTML.to_tree()
           |> length() == 1

    for name <- ["grant[modes][]", "grant[phases][]"] do
      assert grant
             |> LazyHTML.query(~s(select[multiple][name="#{name}"]))
             |> LazyHTML.to_tree()
             |> length() == 1
    end

    assert grant
           |> LazyHTML.query(~s(input[name="grant[destinations]"]))
           |> LazyHTML.to_tree()
           |> length() == 1

    assert LazyHTML.text(grant) =~ "Review and offer grant"
  end

  defp project,
    do: %{
      "id" => "project-1",
      "name" => "Forge",
      "organization_id" => "org-1",
      "organization_name" => "Acme"
    }

  defp secret,
    do: %{
      "id" => "secret-1",
      "name" => "registry-token",
      "status" => "active",
      "active_version_id" => "version-1",
      "grant_count" => 0,
      "can_purge" => false
    }
end
