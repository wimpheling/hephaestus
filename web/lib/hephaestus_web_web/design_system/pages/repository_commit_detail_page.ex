defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryCommitDetailPage do
  @moduledoc "Pure, escaped presentation for an immutable repository commit."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]
  attr :state, :atom, required: true, values: @states
  attr :model, :map, required: true

  def repository_commit_detail(assigns) do
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
      <.frame :if={@model.commit_detail} as="section" id="repository-commit-detail" variant={:summary}>
        <.frame variant={:summary_header}>
          <.frame variant={:resource_detail}>
            <.text as="strong">{commit(@model.commit_detail, "subject")}</.text>
            <.text as="code" variant={:mono}>{commit(@model.commit_detail, "id")}</.text>
          </.frame>
          <.action
            interaction={:navigate}
            variant={:secondary}
            destination={commits_destination(@model)}
          >
            Back to commits
          </.action>
        </.frame>
        <.frame variant={:metadata}>
          <.text as="span" variant={:muted}>Author</.text>
          <.text as="span">{commit(@model.commit_detail, "author_name")}</.text>
          <.text as="span" variant={:muted}>Committed by</.text>
          <.text as="span">{@model.commit_detail["committer_name"] || "Unavailable"}</.text>
          <.text as="span" variant={:muted}>Parent</.text>
          <.text as="code" variant={:mono}>
            {@model.commit_detail["selected_parent"] || "Root commit"}
          </.text>
          <.text as="span" variant={:muted}>All parents</.text>
          <.text as="code" variant={:mono}>{parent_list(@model.commit_detail)}</.text>
        </.frame>
        <.text :if={body(@model.commit_detail) != ""} as="pre" variant={:body}>
          {body(@model.commit_detail)}
        </.text>
        <.page_state
          :if={@model.commit_detail["truncated"]}
          id="commit-diff-truncated"
          state={:reconnecting}
          title="Change list truncated"
          message="Only the first bounded set of changed files is available for this commit."
        />
        <.frame
          :for={file <- @model.commit_detail["files"] || []}
          as="section"
          id={file_id(file)}
          variant={:artifact_panel}
        >
          <.frame variant={:summary_header}>
            <.frame variant={:resource_detail}>
              <.text as="strong">{file["path"]}</.text>
              <.text :if={file["previous_path"] != ""} as="small" variant={:muted}>
                renamed from {file["previous_path"]}
              </.text>
            </.frame>
            <.text as="small" variant={:muted}>{file_summary(file)}</.text>
          </.frame>
          <.page_state
            :if={file["binary"] || diff_state(file) in ["BINARY", "TRUNCATED", "UNAVAILABLE"]}
            id={"#{file_id(file)}-state"}
            state={:empty}
            title="Diff unavailable"
            message="This file is binary, too large, or deliberately bounded."
          />
          <.frame
            :for={{hunk, hunk_index} <- Enum.with_index(file["hunks"] || [], 1)}
            variant={:file_viewer}
          >
            <.text as="code" variant={:mono}>{hunk_header(hunk)}</.text>
            <.source_viewer
              id={"#{file_id(file)}-hunk-#{hunk_index}"}
              aria-label={"Diff for #{file["path"]}, hunk #{hunk_index}"}
              language={language(file["path"])}
              diff_lines={hunk["lines"] || []}
            />
          </.frame>
        </.frame>
      </.frame>
      <.page_state
        :if={!@model.commit_detail && @state == :ready}
        id="commit-detail-unavailable"
        state={:empty}
        title="Commit unavailable"
        message="This commit is no longer reachable from the selected branch."
      />
    </.repository_shell>
    """
  end

  defp commit(detail, key), do: get_in(detail, ["commit", key]) || "Unavailable"
  defp body(detail), do: detail["body"] || ""

  defp commits_destination(%{repository: repository, selected_branch: branch}) do
    suffix = if branch, do: "?ref=#{URI.encode_www_form(branch.name)}", else: ""
    "/repositories/#{repository["id"]}/commits#{suffix}"
  end

  defp file_id(file), do: "commit-file-#{Base.url_encode64(file["path"], padding: false)}"

  defp diff_state(file),
    do: file["state"] |> to_string() |> String.replace_prefix("DIFF_FILE_STATE_", "")

  defp file_summary(file), do: "+#{file["additions"] || 0} −#{file["deletions"] || 0}"

  defp hunk_header(hunk),
    do:
      "@@ -#{hunk["old_start"]},#{hunk["old_lines"]} +#{hunk["new_start"]},#{hunk["new_lines"]} @@"

  defp parent_list(detail) do
    detail
    |> get_in(["commit", "parents"])
    |> List.wrap()
    |> Enum.join(", ")
    |> case do
      "" -> "Root commit"
      value -> value
    end
  end

  defp language(path) do
    case Path.extname(path) do
      ".ex" -> "elixir"
      ".exs" -> "elixir"
      ".json" -> "json"
      ".md" -> "markdown"
      ".rs" -> "rust"
      ".sh" -> "shell"
      ".sql" -> "sql"
      ".toml" -> "toml"
      ".yaml" -> "yaml"
      ".yml" -> "yaml"
      _unknown -> "text"
    end
  end
end
