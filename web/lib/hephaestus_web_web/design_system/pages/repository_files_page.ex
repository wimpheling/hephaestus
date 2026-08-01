defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryFilesPage do
  @moduledoc "Pure repository files route presentation."

  use Phoenix.Component
  import HephaestusWebWeb.DesignSystem

  @states [:loading, :error, :reconnecting, :ready]
  attr :state, :atom, required: true, values: @states
  attr :model, :map, required: true
  attr :branch_form, :any, required: true
  attr :select_branch_event, :string, required: true, values: ["select-branch"]

  def repository_files(assigns) do
    assigns = assign(assigns, :tree, decorate_tree(assigns.model.tree, assigns.model))

    ~H"""
    <.repository_shell
      state={@state}
      repository={@model.repository}
      tabs={@model.tabs}
      active={:files}
      organization_index_destination={@model.destinations[:organization_index]}
      organization_destination={@model.destinations[:organization]}
      project_destination={@model.destinations[:project]}
    >
      <.empty_repository_push
        :if={@model.branches_empty?}
        model={@model}
      />
      <.repository_browser :if={!@model.branches_empty?} id="repository-files">
        <:navigation>
          <.frame variant={:branch_toolbar}>
            <.form_container
              for={@branch_form}
              id="file-branch-selector"
              change={@select_branch_event}
            >
              <.input
                field={@branch_form[:branch]}
                type="select"
                label="Branch"
                options={@model.branch_options}
                disabled={@model.branches_empty?}
              />
            </.form_container>
            <.text :if={@model.selected_branch} as="code" variant={:mono}>
              {short_sha(@model.selected_branch.commit)}
            </.text>
            <.text as="strong">Files</.text>
            <.tag>{@tree.file_count}</.tag>
          </.frame>
        </:navigation>
        <:tree><.repository_tree tree={@tree} current_path={@model.current_path} /></:tree>
        <:content>
          <.frame variant={:artifact_panel}>
            <.frame :if={@model.file} variant={:summary_header}>
              <.frame variant={:resource_detail}>
                <.text as="strong">{@model.file.entry.path}</.text>
                <.text as="small" variant={:muted}>
                  {@model.file.language} · {format_bytes(@model.file.entry.size)}
                </.text>
              </.frame>
              <.text as="code" variant={:mono}>{short_sha(@model.file.entry.object_id)}</.text>
            </.frame>
            <.text :if={@model.file} as="pre" id="file-contents" variant={:mono}>
              {@model.file.contents}
            </.text>
            <.page_state
              :if={@model.file_error}
              id="file-preview-error"
              state={:error}
              title="Preview unavailable"
              message={@model.file_error}
            />
            <.page_state
              :if={!@model.file && !@model.file_error}
              id="file-viewer-empty"
              state={:empty}
              title="Select a file"
              message="The tree is the exact snapshot at the selected branch head."
            />
          </.frame>
        </:content>
      </.repository_browser>
    </.repository_shell>
    """
  end

  defp empty_repository_push(assigns) do
    ~H"""
    <.frame as="section" id="repository-empty-push" variant={:summary_body}>
      <.page_heading
        eyebrow="Empty repository"
        title="Push your first commit"
        description="This repository has no branches yet. Add an agent.toml at the repository root, then push the default branch to start the validated build journey."
      />

      <.frame as="section" id="repository-remote" variant={:panel}>
        <.text as="strong">Git remote</.text>
        <.text as="small" variant={:muted}>
          This URL contains only the repository identity. It never contains a password or bearer token.
        </.text>
        <.text as="code" variant={:mono}>
          {Map.get(@model, :remote_url) || "Remote URL unavailable"}
        </.text>
      </.frame>

      <.frame as="section" id="repository-push-instructions" variant={:panel}>
        <.text as="strong">First push</.text>
        <.text as="small" variant={:muted}>
          Use a short-lived OIDC bearer token through your approved Git HTTP wrapper. Keep the token in
          HEPHAESTUS_GIT_TOKEN; never put it in the remote URL or commit it to the repository.
        </.text>
        <.text as="pre" variant={:mono}>{push_commands(@model)}</.text>
      </.frame>

      <.frame as="section" id="agent-toml-guidance" variant={:panel}>
        <.text as="strong">agent.toml guidance</.text>
        <.text as="small" variant={:muted}>
          Place this file at the repository root. The server parses and validates the exact pushed commit;
          it must describe the version-2 build and runtime contract before an agent run can be requested.
        </.text>
        <.text as="pre" variant={:mono}>{agent_toml_guidance()}</.text>
      </.frame>
    </.frame>
    """
  end

  defp push_commands(model) do
    branch = Map.get(model, :default_branch) || friendly_ref(model.repository["default_branch"])
    remote = Map.get(model, :remote_url) || "<remote-url>"

    """
    git init -b #{branch}
    git remote add origin #{remote}
    git add agent.toml
    git commit -m "Add agent configuration"
    git -c http.extraHeader="Authorization: Bearer ${HEPHAESTUS_GIT_TOKEN}" push -u origin #{branch}
    """
    |> String.trim()
  end

  defp agent_toml_guidance do
    """
    version = 2

    [agent]
    name = "your-agent"
    key = "your-agent"

    [build]
    # Declare the isolated build command, pinned root image, resources,
    # network policy, artifacts, and push-triggered refs here.
    """
    |> String.trim()
  end

  defp friendly_ref("refs/heads/" <> branch), do: branch
  defp friendly_ref(branch), do: branch

  defp decorate_tree(node, %{repository: nil}), do: node

  defp decorate_tree(node, model) do
    ref = model.selected_branch && model.selected_branch.name

    destination = fn path ->
      suffix = if ref, do: "?ref=#{URI.encode_www_form(ref)}", else: ""
      "/repositories/#{model.repository["id"]}/files/#{path}#{suffix}"
    end

    %{
      node
      | directories: Enum.map(node.directories, &decorate_tree(&1, model)),
        files: Enum.map(node.files, &Map.put(&1, :destination, destination.(&1.path)))
    }
  end

  defp short_sha(value), do: String.slice(value, 0, 10)
  defp format_bytes(nil), do: "unknown size"
  defp format_bytes(bytes) when bytes < 1_024, do: "#{bytes} B"
  defp format_bytes(bytes), do: "#{Float.round(bytes / 1_024, 1)} KiB"
end
