defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectAgentsPage do
  @moduledoc "Pure presentation for project agent instances and imports."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :empty, :error, :reconnecting, :ready]

  attr :state, :atom, required: true, values: @states
  attr :project, :map, default: nil
  attr :project_id, :string, required: true
  attr :item_count, :integer, default: 0
  attr :instances, :any, default: []
  attr :release_catalog, :list, default: []
  attr :form, :map, default: %{}
  attr :organization_index_destination, :string, required: true
  attr :organization_destination, :string, required: true
  attr :instance_destination, :any, required: true
  attr :import_event, :string, required: true, values: ["import-agent"]

  @doc "Renders importable releases and configured project agents."
  def project_agents_page(assigns) do
    ~H"""
    <.page_state
      id="project-agents-page-state"
      state={@state}
      title="Agents unavailable"
      message="Project agents are not ready."
    >
      <.frame variant={:summary_body}>
        <.agent_header
          project={@project}
          project_id={@project_id}
          count={@item_count}
          organization_index_destination={@organization_index_destination}
          organization_destination={@organization_destination}
        />
        <.frame as="section" id="release-agent-catalog" variant={:panel}>
          <.page_heading
            eyebrow="Source-owned immutable releases"
            title="Import a release agent"
            description="Importing creates a project-owned instance."
            level="h2"
          />
          <.text :if={@release_catalog == []} as="p" id="release-catalog-empty" variant={:empty}>
            No published release agents are currently authorized for this project.
          </.text>
          <.frame
            :for={release <- @release_catalog}
            as="article"
            id={"release-catalog-#{release["id"]}"}
            variant={:proposal}
          >
            <.text as="strong">{release["display_name"]}</.text>
            <.form_container
              for={to_form(@form, as: :import)}
              id={"import-agent-#{release["id"]}"}
              submit={@import_event}
            >
              <.input name="import[release_agent_id]" type="hidden" value={release["id"]} />
              <.input name="import[name]" value="" label="Instance name" required />
              <.input
                :for={parameter <- release["parameter_schema"] || []}
                name={"import[parameters][#{parameter["name"]}]"}
                value={to_string(parameter["default"] || "")}
                type={if(parameter["sensitive"], do: "password", else: parameter_type(parameter))}
                options={parameter_options(parameter)}
                label={parameter["name"]}
                required={parameter["required"]}
                autocomplete={if(parameter["sensitive"], do: "new-password", else: nil)}
              />
              <.input
                name="import[vcpus]"
                value={
                  to_string(get_in(release, ["runtime_contract", "policy_ceiling", "vcpus"]) || 1)
                }
                type="number"
                label="Virtual CPUs"
                required
              />
              <.input
                name="import[memory_mib]"
                value={
                  to_string(
                    get_in(release, ["runtime_contract", "policy_ceiling", "memory_mib"]) ||
                      512
                  )
                }
                type="number"
                label="Memory MiB"
                required
              />
              <.input
                name="import[network]"
                value={
                  get_in(release, ["runtime_contract", "policy_ceiling", "network"]) ||
                    "disabled"
                }
                type="select"
                label="Network"
                options={[{"Disabled", "disabled"}, {"Enabled", "enabled"}]}
              />
              <.action interaction={:submit} variant={:primary}>Import as new instance</.action>
            </.form_container>
          </.frame>
        </.frame>
        <.frame as="section" id="project-agents" variant={:table}>
          <.resource_list id="project-instance-stream" layout={:projects} update="stream">
            <:header>
              <.text as="span" variant={:muted}>Instance</.text>
              <.text as="span" variant={:muted}>Release</.text>
              <.text as="span" variant={:muted}>Attachments</.text>
              <.text as="span" variant={:muted}>Runs</.text>
            </:header>
            <:empty>No configured agent instances yet.</:empty>
            <.action
              :for={{dom_id, instance} <- @instances}
              id={dom_id}
              destination={@instance_destination.(instance["id"])}
              variant={:resource_row}
            >
              <.text as="strong">{instance["name"]}</.text>
              <.tag>{instance["release_version"] || "unresolved"}</.tag>
              <.text as="span">{instance["attachment_count"]}</.text>
              <.text as="span">{instance["run_count"]}</.text>
            </.action>
          </.resource_list>
        </.frame>
      </.frame>
    </.page_state>
    """
  end

  attr :project, :map, required: true
  attr :project_id, :string, required: true
  attr :count, :integer, required: true
  attr :organization_index_destination, :string, required: true
  attr :organization_destination, :string, required: true
  defp agent_header(assigns), do: project_header(assigns, :agents)

  defp project_header(assigns, active) do
    assigns = assign(assigns, :active, active)

    ~H"""
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
      description="Project-owned agent instances keep independent revisions, state, and runs."
    >
      <:actions>
        <.tag>{@count} visible</.tag>
      </:actions>
    </.page_heading>
    <.tab_navigation id="project-tabs" label="Project" active={@active} items={tabs(@project_id)} />
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
      %{
        key: :builders,
        label: "Builders",
        icon: "hero-cube",
        destination: "/projects/#{id}/builders"
      },
      %{key: :runs, label: "Runs", icon: "hero-play-circle", destination: "/projects/#{id}/runs"},
      %{
        key: :settings,
        label: "Settings",
        icon: "hero-cog-6-tooth",
        destination: "/projects/#{id}/settings"
      }
    ]

  defp parameter_type(%{"value_type" => %{"type" => "enum"}}), do: "select"
  defp parameter_type(%{"value_type" => %{"type" => "integer"}}), do: "number"
  defp parameter_type(%{"type" => "integer"}), do: "number"
  defp parameter_type(_parameter), do: "text"

  defp parameter_options(%{"value_type" => %{"type" => "enum", "values" => values}}),
    do: Enum.map(values, &{&1, &1})

  defp parameter_options(_parameter), do: []
end
