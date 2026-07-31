defmodule HephaestusWebWeb.DesignSystem.Pages.OrganizationSecretsPage do
  @moduledoc "Pure presentation for organization-owned secret authority."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :organization, :map, default: nil
  attr :secrets, :list, default: []
  attr :grants, :list, default: []
  attr :rotate_secret_event, :string, required: true, values: ["rotate-secret"]
  attr :revoke_secret_event, :string, required: true, values: ["revoke-secret"]
  attr :set_secret_enabled_event, :string, required: true, values: ["set-secret-enabled"]
  attr :purge_secret_event, :string, required: true, values: ["purge-secret"]

  @doc "Renders organization-owned secrets and their bounded grants."
  def organization_secrets_page(assigns) do
    ~H"""
    <.page_state
      id="organization-secrets-page-state"
      state={@state}
      title="Secrets unavailable"
      message="Organization secret authority is not ready."
    >
      <.frame variant={:summary_body}>
        <.organization_header organization={@organization} active={:secrets} />
        <.frame as="section" id="owned-secrets-heading" variant={:workspace_heading}>
          <.frame variant={:summary_body}>
            <.text as="p" variant={:eyebrow}>Organization custody</.text>
            <.text as="h2" variant={:title}>Secrets</.text>
            <.text as="p" variant={:lede}>
              Values stay write-only. Bounded grants delegate use without disclosing plaintext.
            </.text>
          </.frame>
          <.frame variant={:page_heading_actions}>
            <.action
              destination={"/organizations/#{@organization["id"]}/secrets/new"}
              variant={:primary}
              test_id="create-organization-secret-link"
            >
              <.glyph name="hero-plus" /> Create organization secret
            </.action>
          </.frame>
        </.frame>

        <.resource_list
          id="organization-secrets"
          layout={:secrets}
          aria_label="Owned organization secrets"
        >
          <:header>
            <.text as="span" variant={:muted}>Owned secret</.text>
            <.text as="span" variant={:muted}>Status</.text>
            <.text as="span" variant={:muted}>Authority</.text>
            <.text as="span" variant={:muted}>Controls</.text>
          </:header>
          <:empty :if={@secrets == []}>No visible organization-owned secrets.</:empty>
          <:row :for={secret <- @secrets}>
            <.frame
              as="article"
              id={"organization-secret-#{secret["id"]}"}
              variant={:resource_row_tall}
            >
              <.frame as="span" variant={:resource_primary}>
                <.frame as="i" variant={:repository_icon}><.glyph name="hero-key" /></.frame>
                <.frame as="span" variant={:resource_detail}>
                  <.text as="strong">{secret["name"]}</.text>
                  <.text as="small">
                    version {secret["active_version_sequence"]} · value unavailable by design
                  </.text>
                </.frame>
              </.frame>
              <.tag tone={secret_tone(secret["status"])}>{secret["status"]}</.tag>
              <.text as="span">{Enum.join(secret["allowed_delivery_modes"], ", ")}</.text>
              <.frame as="span" variant={:resource_controls}>
                <.action
                  :if={secret["status"] in ["active", "disabled"]}
                  interaction={:event}
                  event={@set_secret_enabled_event}
                  event_payload={
                    %{secret_id: secret["id"], enabled: to_string(secret["status"] == "disabled")}
                  }
                  variant={:compact}
                >
                  {if(secret["status"] == "disabled", do: "Enable", else: "Disable")}
                </.action>
                <.form_container
                  :if={secret["can_rotate"] && secret["status"] in ["active", "disabled"]}
                  for={to_form(%{}, as: :rotate)}
                  id={"rotate-organization-secret-#{secret["id"]}"}
                  submit={@rotate_secret_event}
                  layout={:inline}
                >
                  <.input name="rotate[secret_id]" type="hidden" value={secret["id"]} />
                  <.input
                    name="rotate[active_version_id]"
                    type="hidden"
                    value={secret["active_version_id"]}
                  />
                  <.input
                    name="rotate[value]"
                    type="password"
                    value=""
                    placeholder="Replacement value"
                    required
                    autocomplete="new-password"
                  />
                  <.action interaction={:submit} variant={:compact}>Rotate</.action>
                </.form_container>
                <.action
                  :if={secret["can_revoke"] && secret["status"] in ["active", "disabled"]}
                  interaction={:event}
                  event={@revoke_secret_event}
                  event_payload={%{secret_id: secret["id"]}}
                  confirm="Revoke this secret and every downstream authority?"
                  variant={:danger_compact}
                >
                  Revoke
                </.action>
                <.action
                  :if={secret["can_purge"] && secret["status"] in ["revoked", "tombstoned"]}
                  interaction={:event}
                  event={@purge_secret_event}
                  event_payload={%{secret_id: secret["id"]}}
                  confirm="Permanently purge encrypted material?"
                  variant={:danger_compact}
                >
                  Purge
                </.action>
              </.frame>
            </.frame>
          </:row>
        </.resource_list>

        <.frame as="section" id="bounded-grants-heading" variant={:list_subheading}>
          <.frame variant={:summary_body}>
            <.text as="p" variant={:eyebrow}>Delegated authority</.text>
            <.text as="h2" variant={:title}>Bounded grants</.text>
          </.frame>
          <.frame variant={:page_heading_actions}>
            <.tag>{length(@grants)} grants</.tag>
            <.action
              destination={"/organizations/#{@organization["id"]}/secret-grants/new"}
              variant={:primary}
              test_id="offer-organization-grant-link"
            >
              Offer grant
            </.action>
          </.frame>
        </.frame>
        <.resource_list
          id="organization-secret-grants"
          layout={:projects}
          aria_label="Bounded organization grants"
        >
          <:header>
            <.text as="span" variant={:muted}>Secret</.text>
            <.text as="span" variant={:muted}>Target</.text>
            <.text as="span" variant={:muted}>Status</.text>
          </:header>
          <:empty :if={@grants == []}>No bounded grants offered.</:empty>
          <:row :for={grant <- @grants}>
            <.frame as="article" id={"organization-grant-#{grant["id"]}"} variant={:resource_row}>
              <.text as="strong">{grant["secret_name"]}</.text>
              <.text as="span">{grant["target_name"]}</.text>
              <.tag>{grant["status"]}</.tag>
            </.frame>
          </:row>
        </.resource_list>
      </.frame>
    </.page_state>
    """
  end

  defp secret_tone("active"), do: "success"
  defp secret_tone("disabled"), do: "warning"
  defp secret_tone(_status), do: "danger"
end
