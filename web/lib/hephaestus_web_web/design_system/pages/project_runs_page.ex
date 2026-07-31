defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectRunsPage do
  @moduledoc "Pure presentation for project runs."
  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem
  @states [:loading, :empty, :error, :reconnecting, :ready]
  attr :state, :atom, required: true, values: @states
  attr :project, :map, default: nil
  attr :project_id, :string, required: true
  attr :item_count, :integer, default: 0
  attr :runs, :any, default: []
  attr :organization_index_destination, :string, required: true
  attr :organization_destination, :string, required: true
  attr :run_destination, :any, required: true

  @doc "Renders the project run collection."
  def project_runs_page(assigns) do
    ~H"""
    <.page_state
      id="project-runs-page-state"
      state={@state}
      title="Runs unavailable"
      message="Project runs are not ready."
    >
      <.frame variant={:summary_body}>
        <.breadcrumbs id="project-breadcrumbs">
          <:item navigate={@organization_index_destination}>Organizations</:item><:item navigate={
            @organization_destination
          }>
            {@project["organization_name"]}
          </:item><:current>{@project["name"]}</:current>
        </.breadcrumbs>
        <.page_heading
          eyebrow="Project workspace"
          title={@project["name"]}
          description="Exact runs for project-owned agent instances."
        >
          <:actions>
            <.tag>{@item_count} visible</.tag>
          </:actions>
        </.page_heading>
        <.tab_navigation id="project-tabs" label="Project" active={:runs} items={tabs(@project_id)} />
        <.frame as="section" id="project-runs" variant={:table}>
          <.resource_list id="project-run-stream" layout={:projects} update="stream">
            <:header>
              <.text as="span" variant={:muted}>Run</.text><.text as="span" variant={:muted}>
                Repository
              </.text><.text as="span" variant={:muted}>Release</.text><.text
                as="span"
                variant={:muted}
              >
                State
              </.text>
            </:header>
            <:empty>No exact runs have been created.</:empty>
            <.action
              :for={{dom_id, run} <- @runs}
              id={dom_id}
              destination={@run_destination.(run["id"])}
              variant={:text}
            >
              <.text as="strong">{run["instance_name"]}</.text><.text as="span">
                {run["repository_name"]}
              </.text><.text as="span">{run["release_version"]}</.text><.tag>
                {run["outcome"] || run["state"]}
              </.tag>
            </.action>
          </.resource_list>
        </.frame>
      </.frame>
    </.page_state>
    """
  end

  defp tabs(id),
    do: [
      %{
        key: :repositories,
        label: "Repositories",
        icon: "hero-circle-stack",
        destination: "/projects/#{id}"
      },
      %{
        key: :agents,
        label: "Agents",
        icon: "hero-cpu-chip",
        destination: "/projects/#{id}/agents"
      },
      %{key: :runs, label: "Runs", icon: "hero-play-circle", destination: "/projects/#{id}/runs"},
      %{
        key: :settings,
        label: "Settings",
        icon: "hero-cog-6-tooth",
        destination: "/projects/#{id}/settings"
      }
    ]
end
