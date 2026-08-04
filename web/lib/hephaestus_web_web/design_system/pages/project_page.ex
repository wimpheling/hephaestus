defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectPage do
  @moduledoc "Pure presentation for project repositories."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :project, :map, default: nil
  attr :project_id, :string, required: true
  attr :item_count, :integer, default: 0
  attr :repositories, :any, default: []
  attr :organization_index_destination, :string, required: true
  attr :organization_destination, :string, required: true
  attr :repository_destination, :any, required: true

  @doc "Renders the project repository collection."
  def project_page(assigns) do
    ~H"""
    <.page_state
      id="project-repositories-page-state"
      state={@state}
      title="Repositories unavailable"
      message="The project repository collection is not ready."
    >
      <.frame variant={:summary_body}>
        <.project_header
          project={@project}
          project_id={@project_id}
          item_count={@item_count}
          active={:repositories}
          organization_index_destination={@organization_index_destination}
          organization_destination={@organization_destination}
        />
        <.frame as="section" id="project-repositories" variant={:table}>
          <.resource_list id="project-repository-stream" layout={:projects} update="stream">
            <:header>
              <.text as="span" variant={:muted}>Repository</.text>
              <.text as="span" variant={:muted}>Branch</.text>
              <.text as="span" variant={:muted}>Agents</.text>
              <.text as="span" variant={:muted}>Runs</.text>
            </:header>
            <:empty>No visible repositories in this project.</:empty>
            <.action
              :for={{dom_id, repository} <- @repositories}
              id={dom_id}
              destination={@repository_destination.(repository["id"])}
              variant={:text}
            >
              <.frame variant={:resource_primary}>
                <.glyph name="hero-command-line" />
                <.frame variant={:resource_detail}>
                  <.text as="strong">{repository["name"]}</.text>
                  <.text as="small" variant={:muted}>
                    {if(repository["is_public"], do: "public", else: "private")}
                  </.text>
                </.frame>
              </.frame>
              <.text as="code" variant={:mono}>{repository["default_branch"]}</.text>
              <.text as="span">{repository["attachment_count"]}</.text>
              <.text as="span">{repository["run_count"]}</.text>
            </.action>
          </.resource_list>
        </.frame>
      </.frame>
    </.page_state>
    """
  end

  attr :project, :map, required: true
  attr :project_id, :string, required: true
  attr :item_count, :integer, required: true

  attr :active, :atom,
    required: true,
    values: [:repositories, :agents, :builders, :runs, :settings]

  attr :organization_index_destination, :string, required: true
  attr :organization_destination, :string, required: true

  defp project_header(assigns) do
    ~H"""
    <.breadcrumbs id="project-breadcrumbs">
      <:item navigate={@organization_index_destination}>Organizations</:item>
      <:item navigate={@organization_destination}>{@project["organization_name"]}</:item>
      <:current>{@project["name"]}</:current>
    </.breadcrumbs>
    <.page_heading
      eyebrow="Project workspace"
      title={@project["name"]}
      description="Project-owned agent instances keep independent revisions, state, and runs."
    >
      <:actions>
        <.tag>{@item_count} visible</.tag>
        <.action
          id="create-repository-link"
          destination={"/projects/#{@project_id}/repositories/new"}
          variant={:secondary}
        >
          <.glyph name="hero-plus" /> Create repository
        </.action>
      </:actions>
    </.page_heading>
    <.tab_navigation id="project-tabs" label="Project" active={@active} items={tabs(@project_id)} />
    """
  end

  defp tabs(project_id) do
    [
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
        key: :builders,
        label: "Builders",
        icon: "hero-cube",
        destination: "/projects/#{project_id}/builders"
      },
      %{
        key: :runs,
        label: "Runs",
        icon: "hero-play-circle",
        destination: "/projects/#{project_id}/runs"
      },
      %{
        key: :settings,
        label: "Settings",
        icon: "hero-cog-6-tooth",
        destination: "/projects/#{project_id}/settings"
      }
    ]
  end
end
