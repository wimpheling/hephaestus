defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryAgentsPage do
  @moduledoc "Pure repository agents route presentation."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]
  attr :state, :atom, required: true, values: @states
  attr :model, :map, required: true
  attr :attachments, :any, required: true

  def repository_agents(assigns) do
    ~H"""
    <.repository_shell
      state={@state}
      repository={@model.repository}
      tabs={@model.tabs}
      active={:agents}
      organization_index_destination={@model.destinations[:organization_index]}
      organization_destination={@model.destinations[:organization]}
      project_destination={@model.destinations[:project]}
    >
      <.frame as="section" id="repository-agents" variant={:repository_list}>
        <.frame variant={:table_head}>
          <.text as="span">Project instance</.text><.text as="span">Ref selector</.text><.text as="span">
            Release
          </.text>
        </.frame>
        <.frame id="attached-instances" variant={:summary_body} phx_update="stream">
          <.page_state
            :if={@model.attached_instances_empty?}
            id="attached-instances-empty"
            state={:empty}
            title="No attached agents"
            message="No project agent instances are attached to this repository."
          />
          <.frame
            :for={{dom_id, attachment} <- @attachments}
            as="article"
            id={dom_id}
            variant={:table_row}
          >
            <.frame variant={:resource_detail}>
              <.action destination={instance_destination(attachment)}>
                {attachment["instance_name"]}
              </.action>
              <.text as="small" variant={:muted}>{attachment["project_name"]}</.text>
            </.frame>
            <.frame variant={:resource_detail}>
              <.text as="code" variant={:mono}>{attachment["ref_selector"]}</.text>
              <.text as="small" variant={:muted}>{attachment["trigger_policy"]}</.text>
            </.frame>
            <.frame variant={:resource_detail}>
              <.text as="strong">{attachment["release_version"]}</.text>
              <.text as="small" variant={:muted}>{attachment["instance_state"]}</.text>
            </.frame>
          </.frame>
        </.frame>
      </.frame>
    </.repository_shell>
    """
  end

  defp instance_destination(attachment),
    do: "/projects/#{attachment["project_id"]}/agents/#{attachment["instance_id"]}"
end
