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
      <.repository_browser id="repository-files">
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
