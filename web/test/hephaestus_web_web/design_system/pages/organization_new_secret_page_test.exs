defmodule HephaestusWebWeb.DesignSystem.Pages.OrganizationNewSecretPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.OrganizationNewSecretPage
  alias HephaestusWebWeb.OrganizationNewSecretState

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
    assert @covered_statuses == OrganizationNewSecretState.statuses()
    organization = %{"id" => "organization-1", "name" => "Acme"}

    for status <- OrganizationNewSecretState.statuses() do
      state = OrganizationNewSecretState.new(%{organization_id: "organization-1"})
      state = %{state | status: status, data: %{state.data | organization: organization}}
      presentation = OrganizationNewSecretState.present(state)

      html =
        render_component(&OrganizationNewSecretPage.organization_new_secret_page/1,
          state: presentation.status,
          organization: presentation.organization,
          form: presentation.form,
          create_secret_event: "create-secret"
        )

      assert html != ""
    end

    assert length(@covered_states) == 5
  end

  test "preserves the established write-only form contract" do
    state = OrganizationNewSecretState.new(%{organization_id: "organization-1"})

    {ready, []} =
      OrganizationNewSecretState.reduce(
        state,
        {:loaded, %{"id" => "organization-1", "name" => "Acme"}, []}
      )

    presentation = OrganizationNewSecretState.present(ready)

    html =
      render_component(&OrganizationNewSecretPage.organization_new_secret_page/1,
        state: presentation.status,
        organization: presentation.organization,
        form: presentation.form,
        create_secret_event: "create-secret"
      )

    assert html =~ ~s(id="create-organization-secret")
    assert html =~ "Secret name"
    assert html =~ "New value"
    assert html =~ "Allowed delivery modes"
    assert html =~ ~s(name="secret[name]")
    assert html =~ ~s(name="secret[value]")
    assert html =~ ~s(name="secret[modes][]")
    assert html =~ "multiple"
    assert html =~ "Brokered"
    assert html =~ "Raw"
    assert html =~ "Encrypt and create"
    refute html =~ "secret-value"

    document = LazyHTML.from_fragment(html)

    assert document
           |> LazyHTML.query(~s(select[multiple][name="secret[modes][]"]))
           |> LazyHTML.to_tree()
           |> length() == 1
  end
end
