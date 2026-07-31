defmodule HephaestusWebWeb.DesignSystem.Pages.OrganizationNewGrantPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.OrganizationNewGrantPage
  alias HephaestusWebWeb.OrganizationNewGrantState

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
    assert @covered_statuses == OrganizationNewGrantState.statuses()
    organization = %{"id" => "organization-1", "name" => "Acme"}

    for status <- OrganizationNewGrantState.statuses() do
      state = OrganizationNewGrantState.new(%{organization_id: "organization-1"})
      state = %{state | status: status, data: %{state.data | organization: organization}}
      presentation = OrganizationNewGrantState.present(state)

      html =
        render_component(&OrganizationNewGrantPage.organization_new_grant_page/1,
          state: presentation.status,
          organization: presentation.organization,
          form: presentation.form,
          secrets: presentation.secrets,
          projects: presentation.projects,
          repositories: presentation.repositories,
          grant_secret_event: "grant-secret"
        )

      assert html != ""

      if status == :ready do
        document = LazyHTML.from_fragment(html)

        assert document
               |> LazyHTML.query(~s(select[multiple][name="grant[modes][]"]))
               |> LazyHTML.to_tree()
               |> length() == 1

        assert document
               |> LazyHTML.query(~s(select[multiple][name="grant[phases][]"]))
               |> LazyHTML.to_tree()
               |> length() == 1
      end
    end

    assert length(@covered_states) == 5
  end

  test "renders the prompt before each loaded secret option" do
    state = OrganizationNewGrantState.new(%{organization_id: "organization-1"})
    organization = %{"id" => "organization-1", "name" => "Acme"}
    secret = %{"id" => "secret-1", "name" => "organization_token", "status" => "active"}

    {ready, []} =
      OrganizationNewGrantState.reduce(
        state,
        {:loaded, organization, [secret], [], []}
      )

    presentation = OrganizationNewGrantState.present(ready)

    html =
      render_component(&OrganizationNewGrantPage.organization_new_grant_page/1,
        state: presentation.status,
        organization: presentation.organization,
        form: presentation.form,
        secrets: presentation.secrets,
        projects: presentation.projects,
        repositories: presentation.repositories,
        grant_secret_event: "grant-secret"
      )

    document = LazyHTML.from_fragment(html)
    select = LazyHTML.query(document, ~s(select[name="grant[secret_id]"]))

    assert LazyHTML.text(LazyHTML.query(select, "option:first-child")) == "Choose a secret"
    assert LazyHTML.text(LazyHTML.query(select, "option:nth-child(2)")) == "organization_token"

    assert select
           |> LazyHTML.query(~s(option[value="secret-1"]))
           |> LazyHTML.to_tree()
           |> length() == 1

    assert html =~ "Offer exact grant"
  end
end
