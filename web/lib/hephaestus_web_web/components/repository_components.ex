defmodule HephaestusWebWeb.RepositoryComponents do
  @moduledoc """
  Components shared by repository browsing views.
  """

  use HephaestusWebWeb, :html

  attr :repository_id, :string, required: true

  attr :active, :atom,
    required: true,
    values: [:files, :commits, :branches, :releases, :agents]

  attr :branch, :string, default: nil

  def repository_tabs(assigns) do
    ~H"""
    <nav id="repository-tabs" class="repository-tabs" aria-label="Repository">
      <.link
        navigate={with_ref(~p"/repositories/#{@repository_id}/files", @branch)}
        class={["repository-tab", @active == :files && "active"]}
        aria-current={if(@active == :files, do: "page")}
      >
        <.icon name="hero-folder" class="size-4" /> Files
      </.link>
      <.link
        navigate={with_ref(~p"/repositories/#{@repository_id}/commits", @branch)}
        class={["repository-tab", @active == :commits && "active"]}
        aria-current={if(@active == :commits, do: "page")}
      >
        <.icon name="hero-clock" class="size-4" /> Commits
      </.link>
      <.link
        navigate={~p"/repositories/#{@repository_id}/branches"}
        class={["repository-tab", @active == :branches && "active"]}
        aria-current={if(@active == :branches, do: "page")}
      >
        <.icon name="hero-code-bracket" class="size-4" /> Branches
      </.link>
      <.link
        navigate={~p"/repositories/#{@repository_id}/releases"}
        class={["repository-tab", @active == :releases && "active"]}
        aria-current={if(@active == :releases, do: "page")}
      >
        <.icon name="hero-cube-transparent" class="size-4" /> Releases
      </.link>
      <.link
        navigate={~p"/repositories/#{@repository_id}/agents"}
        class={["repository-tab", @active == :agents && "active"]}
        aria-current={if(@active == :agents, do: "page")}
      >
        <.icon name="hero-cpu-chip" class="size-4" /> Agents
      </.link>
    </nav>
    """
  end

  attr :tree, :map, required: true
  attr :repository_id, :string, required: true
  attr :branch, :string, default: nil
  attr :current_path, :string, default: nil

  def file_tree(assigns) do
    ~H"""
    <div id="repository-file-tree" class="file-tree">
      <.tree_directory
        node={@tree}
        repository_id={@repository_id}
        branch={@branch}
        current_path={@current_path}
      />
      <div :if={@tree.directories == [] and @tree.files == []} class="file-tree-empty">
        This branch has no files.
      </div>
    </div>
    """
  end

  attr :node, :map, required: true
  attr :repository_id, :string, required: true
  attr :branch, :string, required: true
  attr :current_path, :string, default: nil

  defp tree_directory(assigns) do
    ~H"""
    <div class="tree-level">
      <details
        :for={directory <- @node.directories}
        id={"tree-directory-#{tree_id(directory.path)}"}
        open={directory_open?(directory.path, @current_path)}
      >
        <summary>
          <.icon name="hero-chevron-right" class="tree-chevron size-3" />
          <.icon name="hero-folder" class="size-4" />
          <span>{directory.name}</span>
        </summary>
        <.tree_directory
          node={directory}
          repository_id={@repository_id}
          branch={@branch}
          current_path={@current_path}
        />
      </details>
      <.link
        :for={file <- @node.files}
        id={"tree-file-#{tree_id(file.path)}"}
        navigate={file_url(@repository_id, @branch, file.path)}
        class={["tree-file", file.path == @current_path && "active"]}
        title={file.path}
      >
        <.icon name={file_icon(file)} class="size-4" />
        <span>{file.name}</span>
      </.link>
    </div>
    """
  end

  defp file_url(repository_id, branch, path) do
    segments = String.split(path, "/", trim: true)
    ~p"/repositories/#{repository_id}/files/#{segments}?#{[ref: branch]}"
  end

  defp file_icon(%{mode: "120000"}), do: "hero-link"
  defp file_icon(_file), do: "hero-document"

  defp directory_open?(_directory_path, nil), do: false

  defp directory_open?(directory_path, current_path),
    do: String.starts_with?(current_path, directory_path <> "/")

  defp tree_id(path) do
    :crypto.hash(:sha256, path)
    |> Base.url_encode64(padding: false)
    |> binary_part(0, 12)
  end

  defp with_ref(path, nil), do: path
  defp with_ref(path, branch), do: "#{path}?#{URI.encode_query(%{"ref" => branch})}"
end
