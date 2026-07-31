defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryReleasesPage do
  @moduledoc "Pure repository releases route presentation."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]
  attr :state, :atom, required: true, values: @states
  attr :model, :map, required: true
  attr :releases, :any, required: true

  def repository_releases(assigns) do
    ~H"""
    <.repository_shell
      state={@state}
      repository={@model.repository}
      tabs={@model.tabs}
      active={:releases}
      organization_index_destination={@model.destinations[:organization_index]}
      organization_destination={@model.destinations[:organization]}
      project_destination={@model.destinations[:project]}
    >
      <.frame as="section" id="repository-releases" variant={:repository_list}>
        <.frame variant={:table_head}>
          <.text as="span">Release</.text><.text as="span">Source</.text><.text as="span">
            Artifacts
          </.text>
        </.frame>
        <.frame id="releases" variant={:summary_body} phx_update="stream">
          <.page_state
            :if={@model.releases_empty?}
            id="releases-empty"
            state={:empty}
            title="No releases"
            message="No immutable releases have been built from this repository."
          />
          <.frame :for={{dom_id, release} <- @releases} as="article" id={dom_id} variant={:table_row}>
            <.frame variant={:resource_primary}>
              <.action destination={release_destination(@model.repository, release["id"])}>
                {release["version"]}
              </.action>
              <.tag tone={release_tone(release["state"])}>{release["state"]}</.tag>
            </.frame>
            <.frame variant={:resource_detail}>
              <.text as="code" variant={:mono}>{String.slice(release["source_commit"], 0, 10)}</.text>
              <.text as="small" variant={:muted}>{friendly_ref(release["source_ref"])}</.text>
            </.frame>
            <.frame variant={:resource_detail}>
              <.text as="strong">{release["artifact_count"]} artifacts</.text>
              <.text as="small" variant={:muted}>
                {release["exported_agent_count"]} exported agents
              </.text>
            </.frame>
          </.frame>
        </.frame>
      </.frame>
    </.repository_shell>
    """
  end

  defp release_destination(repository, release_id),
    do: "/repositories/#{repository["id"]}/releases/#{release_id}"

  defp release_tone("published"), do: "success"
  defp release_tone("revoked"), do: "danger"
  defp release_tone(_state), do: "neutral"
  defp friendly_ref("refs/heads/" <> branch), do: branch
  defp friendly_ref(git_ref), do: git_ref
end
