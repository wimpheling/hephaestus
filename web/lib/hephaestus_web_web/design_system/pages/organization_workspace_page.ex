defmodule HephaestusWebWeb.DesignSystem.Pages.OrganizationWorkspacePage do
  @moduledoc "Pure presentation for an organization's projects."

  use Phoenix.Component

  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :organization, :map, default: nil
  attr :projects, :list, default: []

  @doc "Renders the organization project collection."
  def organization_workspace_page(assigns) do
    ~H"""
    <.page_state
      id="organization-projects-page-state"
      state={@state}
      title="Projects unavailable"
      message="The organization project collection is not ready."
    >
      <.frame variant={:summary_body}>
        <.organization_header organization={@organization} active={:projects} />
        <.frame as="section" variant={:workspace_heading}>
          <.frame variant={:summary_body}>
            <.text as="p" variant={:eyebrow}>Organization resources</.text>
            <.text as="h2" variant={:title}>Projects</.text>
            <.text as="p" variant={:lede}>
              Reusable agents, repositories, and exact runs grouped by project.
            </.text>
          </.frame>
          <.frame variant={:resource_primary}>
            <.tag>{length(@projects)} projects</.tag>
            <.action
              :if={@organization}
              id="create-project-link"
              destination={"/organizations/#{@organization["id"]}/projects/new"}
              variant={:secondary}
            >
              <.glyph name="hero-plus" /> Create project
            </.action>
          </.frame>
        </.frame>

        <.resource_list id="projects" layout={:projects} aria_label="Projects">
          <:header>
            <.text as="span" variant={:muted}>Project</.text>
            <.text as="span" variant={:muted}>Repositories</.text>
            <.text as="span" variant={:muted}>Agents</.text>
            <.text as="span" variant={:muted}>Runs</.text>
          </:header>
          <:empty :if={@projects == []}>No visible projects.</:empty>
          <:row :for={project <- @projects}>
            <.action
              id={"project-stream-#{project["id"]}"}
              destination={"/projects/#{project["id"]}"}
              variant={:resource_row}
              test_id={"project-#{project["id"]}"}
            >
              <.frame as="span" variant={:resource_primary}>
                <.frame as="i" variant={:repository_icon}>P</.frame>
                <.frame as="span" variant={:resource_detail}>
                  <.text as="strong">{project["name"]}</.text>
                  <.text as="small">{relative_time(project["last_activity_at"])}</.text>
                </.frame>
              </.frame>
              <.text as="span">{project["repository_count"]}</.text>
              <.text as="span">{project["instance_count"]}</.text>
              <.text as="span">{project["run_count"]}</.text>
            </.action>
          </:row>
        </.resource_list>
      </.frame>
    </.page_state>
    """
  end

  defp relative_time(nil), do: "no activity yet"
  defp relative_time(timestamp), do: "updated #{timestamp}"
end
