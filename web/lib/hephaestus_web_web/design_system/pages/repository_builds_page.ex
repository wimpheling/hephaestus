defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryBuildsPage do
  @moduledoc "Pure repository builds route presentation."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]
  attr :state, :atom, required: true, values: @states
  attr :model, :map, required: true
  attr :builds, :any, required: true
  attr :build_request_form, :any, required: true
  attr :request_event, :string, required: true, values: ["request-build"]

  def repository_builds(assigns) do
    ~H"""
    <.repository_shell
      state={@state}
      repository={@model.repository}
      tabs={@model.tabs}
      active={:builds}
      organization_index_destination={@model.destinations[:organization_index]}
      organization_destination={@model.destinations[:organization]}
      project_destination={@model.destinations[:project]}
    >
      <.page_heading
        eyebrow="Agent release builds"
        title="Build history"
        description="Build requests are immutable inputs that produce draft releases."
      />

      <.frame as="section" id="build-request" variant={:panel}>
        <.page_heading
          eyebrow="Manual request"
          title="Build an exact commit"
          description="Supply the hashes from the validated agent.toml build declaration."
          level="h2"
        />
        <.form_container
          for={@build_request_form}
          id="build-request-form"
          as={:build}
          submit={@request_event}
        >
          <.input
            name="build[source_commit]"
            value={@build_request_form[:source_commit].value}
            label="Source commit"
            required
          />
          <.input
            name="build[build_definition_hash]"
            value={@build_request_form[:build_definition_hash].value}
            label="Build definition hash"
            required
          />
          <.input
            name="build[configuration_hash]"
            value={@build_request_form[:configuration_hash].value}
            label="Configuration hash"
            required
          />
          <.action interaction={:submit} variant={:primary}>Request build</.action>
        </.form_container>
      </.frame>

      <.frame as="section" id="repository-builds" variant={:repository_list}>
        <.frame variant={:table_head}>
          <.text as="span">Build</.text><.text as="span">Source</.text><.text as="span">Result</.text>
        </.frame>
        <.frame id="builds" variant={:summary_body} phx_update="stream">
          <.page_state
            :if={@model.builds_unavailable?}
            id="builds-unavailable"
            state={:error}
            title="Build history unavailable"
            message="The typed BuildService is temporarily unavailable for this repository."
          />
          <.page_state
            :if={!@model.builds_unavailable? && @model.builds_empty?}
            id="builds-empty"
            state={:empty}
            title="No builds"
            message="No build records are available for this repository."
          />
          <.frame :for={{dom_id, build} <- @builds} as="article" id={dom_id} variant={:table_row}>
            <.action destination={build_destination(@model.repository, build["id"])}>
              {short_id(build["id"])}
            </.action>
            <.text as="code" variant={:mono}>{short_id(build["source_commit"])}</.text>
            <.tag tone={build_tone(build["state"])}>{build["state"]}</.tag>
          </.frame>
        </.frame>
      </.frame>
    </.repository_shell>
    """
  end

  defp build_destination(repository, build_id),
    do: "/repositories/#{repository["id"]}/builds/#{build_id}"

  defp short_id(nil), do: "—"
  defp short_id(value), do: String.slice(value, 0, 12)
  defp build_tone("succeeded"), do: "success"
  defp build_tone("failed"), do: "danger"
  defp build_tone("running"), do: "warning"
  defp build_tone(_state), do: "neutral"
end
