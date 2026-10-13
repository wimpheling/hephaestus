defmodule HephaestusWebWeb.DesignSystem.Pages.OrganizationNewSecretPage do
  @moduledoc "Pure presentation for creating an organization-owned secret."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :organization, :map, default: nil
  attr :form, :map, default: %{}
  attr :create_secret_event, :string, required: true, values: ["create-secret"]

  attr :write_only_hook, :string, required: true, values: ["OrganizationSecretWriteOnly"]

  @doc "Renders the write-only organization secret form."
  def organization_new_secret_page(assigns) do
    ~H"""
    <.page_state
      id="organization-new-secret-page-state"
      state={@state}
      title="Secret form unavailable"
      message="The organization secret form is not ready."
    >
      <.frame variant={:summary_body}>
        <.organization_header organization={@organization} active={:secrets} />
        <.page_heading
          eyebrow="Organization custody"
          title="Create organization secret"
          description="The plaintext is accepted once and is never returned."
        />
        <div id="organization-secret-write-only" phx-hook={@write_only_hook}>
          <.form_container
            for={to_form(@form, as: :secret)}
            id="create-organization-secret"
            submit={@create_secret_event}
          >
            <.input name="secret[name]" value="" label="Secret name" required />
            <.input
              name="secret[value]"
              value=""
              type="password"
              label="New value"
              required
              autocomplete="new-password"
            />
            <.input
              name="secret[modes][]"
              type="select"
              value={[]}
              label="Allowed delivery modes"
              options={[{"Brokered", "brokered"}, {"Raw", "raw"}]}
              multiple
              required
            />
            <.action interaction={:submit} variant={:primary}>Encrypt and create</.action>
          </.form_container>
        </div>
      </.frame>
    </.page_state>
    """
  end
end
