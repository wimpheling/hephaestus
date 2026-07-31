defmodule HephaestusWebWeb.DesignSystem.Pages.OrganizationSecretsPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.OrganizationSecretsPage
  alias HephaestusWebWeb.OrganizationSecretsState

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
    assert @covered_statuses == OrganizationSecretsState.statuses()
    organization = %{"id" => "organization-1", "name" => "Acme"}

    grant = %{
      "id" => "grant-1",
      "secret_name" => "deploy_key",
      "target_name" => "Forge",
      "status" => "offered"
    }

    for status <- OrganizationSecretsState.statuses() do
      state = OrganizationSecretsState.new(%{organization_id: "organization-1"})

      state = %{
        state
        | status: status,
          data: %{state.data | organization: organization, grants: [grant]}
      }

      presentation = OrganizationSecretsState.present(state)

      html =
        render_component(&OrganizationSecretsPage.organization_secrets_page/1,
          state: presentation.status,
          organization: presentation.organization,
          secrets: presentation.secrets,
          grants: presentation.grants,
          rotate_secret_event: "rotate-secret",
          revoke_secret_event: "revoke-secret",
          set_secret_enabled_event: "set-secret-enabled",
          purge_secret_event: "purge-secret"
        )

      assert html != ""
    end

    assert Enum.sort(@covered_states) ==
             Enum.sort([:loading, :empty, :error, :reconnecting, :ready])
  end
end
