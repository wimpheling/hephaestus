defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectGatewayPage do
  @moduledoc "Pure presentation for an authorized gateway and its ingress audit summary."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :gateway, :map, default: nil
  attr :ingress, :list, default: []
  attr :gateways_destination, :string, required: true
  attr :lifecycle_event, :string, required: true, values: ["lifecycle"]
  attr :lifecycle_actions, :list, default: []

  @doc "Renders a gateway detail view from its presentation model."
  def project_gateway_page(assigns) do
    ~H"""
    <.page_state
      id="project-gateway-page-state"
      state={@state}
      title="Gateway unavailable"
      message="Gateway information is not ready."
    >
      <.frame variant={:summary_body}>
        <.breadcrumbs id="project-gateway-breadcrumbs">
          <:item navigate={@gateways_destination}>Gateways</:item>
          <:current>{@gateway["name"]}</:current>
        </.breadcrumbs>
        <.page_heading
          eyebrow="Project ingress"
          title={@gateway["name"]}
          description="Immutable gateway revisions and authorized ingress outcomes."
        >
          <:actions>
            <.tag>{@gateway["lifecycle"]}</.tag>
            <.action
              :for={action <- @lifecycle_actions}
              interaction={:event}
              event={@lifecycle_event}
              event_payload={%{next: action.next}}
              variant={action.variant}
            >
              {action.label}
            </.action>
          </:actions>
        </.page_heading>
        <.frame as="section" id="gateway-revisions" variant={:table}>
          <.page_heading eyebrow="Immutable revisions" title="Routes" level="h2" />
          <.resource_list id="gateway-revision-list" layout={:projects}>
            <:header>
              <.text as="span" variant={:muted}>Handler contract</.text>
              <.text as="span" variant={:muted}>Routes</.text>
            </:header>
            <:empty>No revisions are available.</:empty>
            <.frame
              :for={revision <- @gateway["revisions"] || []}
              as="article"
              id={"gateway-revision-#{revision["id"] || revision["handler_contract"]}"}
              variant={:table_row}
            >
              <.text as="strong">{revision["handler_contract"]}</.text>
              <.frame variant={:resource_detail}>
                <.text as="small" variant={:muted}>{revision["exposure"]}</.text>
                <.text :for={route <- revision["routes"] || []} as="small">
                  {route["path"]} · {Enum.join(route["methods"] || [], ", ")}
                </.text>
              </.frame>
            </.frame>
          </.resource_list>
        </.frame>
        <.frame as="section" id="gateway-ingress" variant={:table}>
          <.page_heading eyebrow="Ingress audit" title="Recent outcomes" level="h2" />
          <.resource_list id="gateway-ingress-list" layout={:projects}>
            <:header>
              <.text as="span" variant={:muted}>Outcome</.text>
              <.text as="span" variant={:muted}>Accepted at</.text>
            </:header>
            <:empty>No ingress records are available.</:empty>
            <.frame
              :for={entry <- @ingress}
              as="article"
              id={"gateway-ingress-#{entry["id"] || entry["accepted_at"]}"}
              variant={:table_row}
            >
              <.tag>{entry["outcome"]}</.tag>
              <.text as="time">{entry["accepted_at"]}</.text>
            </.frame>
          </.resource_list>
        </.frame>
      </.frame>
    </.page_state>
    """
  end
end
