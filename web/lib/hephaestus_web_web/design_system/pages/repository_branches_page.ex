defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryBranchesPage do
  @moduledoc "Pure repository branches route presentation."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]
  attr :state, :atom, required: true, values: @states
  attr :model, :map, required: true
  attr :branches, :any, required: true

  def repository_branches(assigns) do
    ~H"""
    <.repository_shell
      state={@state}
      repository={@model.repository}
      tabs={@model.tabs}
      active={:branches}
      organization_index_destination={@model.destinations[:organization_index]}
      organization_destination={@model.destinations[:organization]}
      project_destination={@model.destinations[:project]}
    >
      <.frame as="section" id="repository-branches" variant={:repository_list}>
        <.frame variant={:table_head}>
          <.text as="span">Branch</.text><.text as="span">Head commit</.text><.text as="span">
            Updated
          </.text>
        </.frame>
        <.frame id="branches" variant={:summary_body} phx_update="stream">
          <.page_state
            :if={@model.branches_empty?}
            id="branches-empty"
            state={:empty}
            title="No branches"
            message="No branches have been pushed yet."
          />
          <.frame :for={{dom_id, branch} <- @branches} as="article" id={dom_id} variant={:table_row}>
            <.frame variant={:resource_primary}>
              <.action destination={branch_destination(@model.repository, branch.name)}>
                <.glyph name="hero-code-bracket" /> {branch.name}
              </.action>
              <.tag :if={branch.ref == @model.repository["default_branch"]} tone="accent">
                default
              </.tag>
            </.frame>
            <.frame variant={:resource_detail}>
              <.text as="code" variant={:mono}>{String.slice(branch.commit, 0, 10)}</.text>
              <.text as="small" variant={:muted}>{branch.subject}</.text>
            </.frame>
            <.text as="time" datetime={branch.committed_at}>
              {display_time(branch.committed_at)}
            </.text>
          </.frame>
        </.frame>
      </.frame>
    </.repository_shell>
    """
  end

  defp branch_destination(repository, branch),
    do: "/repositories/#{repository["id"]}/files?ref=#{URI.encode_www_form(branch)}"

  defp display_time(value) do
    case DateTime.from_iso8601(value) do
      {:ok, date_time, _offset} -> Calendar.strftime(date_time, "%d %b %Y · %H:%M")
      _error -> value
    end
  end
end
