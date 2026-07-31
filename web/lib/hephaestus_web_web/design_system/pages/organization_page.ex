defmodule HephaestusWebWeb.DesignSystem.Pages.OrganizationPage do
  @moduledoc "Pure presentation for the organization index."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem,
    only: [action: 1, frame: 1, page_heading: 1, page_state: 1, tag: 1, text: 1]

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :current_identity, :map, required: true
  attr :organization_count, :integer, required: true
  attr :organizations, :any, required: true

  @doc "Renders the ready organization index from route-provided presentation data."
  def organization_page(assigns) do
    ~H"""
    <.page_state
      id="organizations-page-state"
      state={@state}
      title="Organizations unavailable"
      message="The organization perimeter is not ready."
    >
      <.frame variant={:summary_body}>
        <.frame as="section" variant={:hero_panel}>
          <.frame variant={:summary_body}>
            <.text as="p" variant={:eyebrow}>Control plane</.text>
            <.text as="h1" variant={:title}>
              Good evening, {@current_identity.display_name}.
            </.text>
            <.text as="p" variant={:lede}>
              Review live agents, inspect exact commits, and decide what reaches Git.
            </.text>
          </.frame>
          <.frame variant={:system_health}>
            <.frame as="span" variant={:status_dot} />
            <.frame variant={:summary_body}>
              <.text as="strong">Forge online</.text>
              <.text as="small">durable workers connected</.text>
            </.frame>
          </.frame>
        </.frame>

        <.page_heading eyebrow="Your perimeter" title="Organizations" level="h2">
          <:actions>
            <.tag>{@organization_count} visible</.tag>
          </:actions>
        </.page_heading>

        <.frame
          id="organizations"
          variant={:organization_grid}
          phx_update="stream"
        >
          <.action
            :for={{dom_id, organization} <- @organizations}
            id={dom_id}
            destination={"/organizations/#{organization["id"]}"}
            variant={:organization_card}
            test_id={"organization-#{organization["id"]}"}
          >
            <.frame variant={:organization_card_mark}>
              {organization["name"] |> String.first() |> String.upcase()}
            </.frame>
            <.frame variant={:organization_card_copy}>
              <.text as="h3" variant={:title}>{organization["name"]}</.text>
              <.text as="p">
                {organization["project_count"]} projects · {organization["repository_count"]} repositories
              </.text>
            </.frame>
            <.frame as="span" variant={:arrow}>↗</.frame>
          </.action>
        </.frame>
      </.frame>
    </.page_state>
    """
  end
end
