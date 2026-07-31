defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryCommitsPage do
  @moduledoc "Pure repository commits route presentation."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]
  attr :state, :atom, required: true, values: @states
  attr :model, :map, required: true
  attr :commits, :any, required: true
  attr :branch_form, :any, required: true
  attr :select_branch_event, :string, required: true, values: ["select-branch"]

  def repository_commits(assigns) do
    ~H"""
    <.repository_shell
      state={@state}
      repository={@model.repository}
      tabs={@model.tabs}
      active={:commits}
      organization_index_destination={@model.destinations[:organization_index]}
      organization_destination={@model.destinations[:organization]}
      project_destination={@model.destinations[:project]}
    >
      <.frame as="section" variant={:branch_toolbar}>
        <.form_container for={@branch_form} id="branch-selector" change={@select_branch_event}>
          <.input
            field={@branch_form[:branch]}
            type="select"
            label="Branch"
            options={@model.branch_options}
            disabled={@model.branches_empty?}
          />
        </.form_container>
        <.frame :if={@model.selected_branch} variant={:resource_detail}>
          <.text as="span" variant={:muted}>Head</.text>
          <.text as="code" variant={:mono}>{short_sha(@model.selected_branch.commit)}</.text>
        </.frame>
      </.frame>
      <.frame as="section" id="repository-commits" variant={:repository_list}>
        <.frame variant={:table_head}>
          <.text as="span">Commit</.text><.text as="span">Author</.text><.text as="span">Date</.text>
        </.frame>
        <.frame id="commits" variant={:summary_body} phx_update="stream">
          <.page_state
            :if={@model.commits_empty?}
            id="commits-empty"
            state={:empty}
            title="No commits"
            message="No commits on this branch."
          />
          <.frame :for={{dom_id, commit} <- @commits} as="article" id={dom_id} variant={:table_row}>
            <.frame variant={:resource_detail}>
              <.text as="strong">{commit.subject}</.text>
              <.text as="code" variant={:mono}>{short_sha(commit.id)}</.text>
            </.frame>
            <.frame variant={:resource_detail}>
              <.text as="strong">{commit.author_name}</.text>
              <.text as="small" variant={:muted}>{commit.author_email}</.text>
            </.frame>
            <.text as="time" datetime={commit.authored_at}>{display_time(commit.authored_at)}</.text>
          </.frame>
        </.frame>
      </.frame>
    </.repository_shell>
    """
  end

  defp short_sha(value), do: String.slice(value, 0, 10)

  defp display_time(value) do
    case DateTime.from_iso8601(value) do
      {:ok, date_time, _offset} -> Calendar.strftime(date_time, "%d %b %Y · %H:%M")
      _error -> value
    end
  end
end
