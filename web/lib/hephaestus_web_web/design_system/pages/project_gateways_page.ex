defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectGatewaysPage do
  @moduledoc "Pure presentation for the project gateway collection."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :project_id, :string, required: true
  attr :gateways, :list, default: []
  attr :gateway_destination, :any, required: true

  @doc "Renders authorized gateway summaries without exposing ingress payloads."
  def project_gateways_page(assigns) do
    ~H"""
    <.page_state
      id="project-gateways-page-state"
      state={@state}
      title="Gateways unavailable"
      message="Gateway information is not ready."
    >
      <.frame variant={:summary_body}>
        <.page_heading
          eyebrow="Project ingress"
          title="Gateways"
          description="Routes, revisions, and ingress outcomes are visible only to authorized project members. Request bodies and secrets are never shown."
        />
        <.tab_navigation
          id="project-tabs"
          label="Project"
          active={:gateways}
          items={tabs(@project_id)}
        />
        <.frame as="section" id="project-gateway-list" variant={:table}>
          <.resource_list id="project-gateway-resources" layout={:projects}>
            <:header>
              <.text as="span" variant={:muted}>Gateway</.text>
              <.text as="span" variant={:muted}>Lifecycle</.text>
            </:header>
            <:empty>No gateways are installed for this project.</:empty>
            <.action
              :for={gateway <- @gateways}
              id={"gateway-#{gateway["id"]}"}
              destination={@gateway_destination.(gateway["id"])}
              variant={:resource_row}
            >
              <.text as="strong">{gateway["name"]}</.text>
              <.tag>{gateway["lifecycle"] || "unspecified"}</.tag>
            </.action>
          </.resource_list>
        </.frame>
      </.frame>
    </.page_state>
    """
  end

  defp tabs(project_id),
    do: [
      %{
        key: :repositories,
        label: "Repositories",
        icon: "hero-circle-stack",
        destination: "/projects/#{project_id}"
      },
      %{
        key: :agents,
        label: "Agents",
        icon: "hero-cpu-chip",
        destination: "/projects/#{project_id}/agents"
      },
      %{
        key: :runs,
        label: "Runs",
        icon: "hero-play-circle",
        destination: "/projects/#{project_id}/runs"
      },
      %{
        key: :images,
        label: "Images",
        icon: "hero-cube",
        destination: "/projects/#{project_id}/images"
      },
      %{
        key: :gateways,
        label: "Gateways",
        icon: "hero-globe-alt",
        destination: "/projects/#{project_id}/gateways"
      },
      %{
        key: :settings,
        label: "Settings",
        icon: "hero-cog-6-tooth",
        destination: "/projects/#{project_id}/settings"
      }
    ]
end
