defmodule HephaestusWebWeb.DesignSystem.Pages.OrganizationNewGrantPage do
  @moduledoc "Pure presentation for offering an organization secret grant."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :organization, :map, default: nil
  attr :form, :map, default: %{}
  attr :secrets, :list, default: []
  attr :projects, :list, default: []
  attr :repositories, :list, default: []
  attr :grant_secret_event, :string, required: true, values: ["grant-secret"]

  @doc "Renders the exact-target bounded-grant form."
  def organization_new_grant_page(assigns) do
    ~H"""
    <.page_state
      id="organization-new-grant-page-state"
      state={@state}
      title="Grant form unavailable"
      message="The bounded-grant form is not ready."
    >
      <.frame variant={:summary_body}>
        <.organization_header organization={@organization} active={:secrets} />
        <.page_heading
          eyebrow="Delegated authority"
          title="Offer a bounded grant"
          description="Grants are exact and non-transitive."
        />
        <.form_container
          for={to_form(@form, as: :grant)}
          id="grant-organization-secret"
          submit={@grant_secret_event}
        >
          <.input
            name="grant[secret_id]"
            value=""
            type="select"
            label="Secret"
            prompt="Choose a secret"
            options={Enum.map(@secrets, &{&1["name"], &1["id"]})}
            required
          />
          <.input
            name="grant[target]"
            value=""
            type="select"
            label="Exact target"
            options={target_options(@projects, @repositories)}
            required
          />
          <.input
            name="grant[modes][]"
            type="select"
            value={[]}
            label="Delivery modes"
            options={[{"Brokered", "brokered"}, {"Raw", "raw"}]}
            multiple
          />
          <.input
            name="grant[phases][]"
            type="select"
            value={[]}
            label="Phases"
            options={[{"Normal", "normal"}, {"Preflight", "preflight"}, {"Update", "update"}]}
            multiple
          />
          <.input name="grant[destinations]" value="" label="Destinations" />
          <.input name="grant[expires_at]" value="" type="datetime-local" label="Expires at" />
          <.action interaction={:submit} variant={:primary}>Offer exact grant</.action>
        </.form_container>
      </.frame>
    </.page_state>
    """
  end

  defp target_options(projects, repositories) do
    Enum.map(projects, &{"Project · #{&1["name"]}", "project:#{&1["id"]}"}) ++
      Enum.map(repositories, &{"Repository · #{&1["name"]}", "repository:#{&1["id"]}"})
  end
end
