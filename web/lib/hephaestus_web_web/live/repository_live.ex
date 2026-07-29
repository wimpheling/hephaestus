defmodule HephaestusWebWeb.RepositoryLive do
  use HephaestusWebWeb, :live_view

  import HephaestusWebWeb.RepositoryComponents

  alias HephaestusWeb.{RepositoryBrowser, RunNotifier, Store}

  @impl true
  def mount(_params, _session, socket) do
    {:ok,
     socket
     |> stream_configure(:commits, dom_id: &"commit-#{&1.id}")
     |> stream_configure(:branches, dom_id: &"branch-#{tree_id(&1.ref)}")
     |> stream_configure(:releases, dom_id: &"release-#{&1["id"]}")
     |> stream_configure(:attached_instances, dom_id: &"attachment-#{&1["id"]}")
     |> assign(:repository, nil)
     |> assign(:selected_branch, nil)
     |> assign(:branch_options, [])
     |> assign(:branch_form, to_form(%{"branch" => ""}, as: :browse))
     |> assign(:branches_empty?, true)
     |> assign(:commits_empty?, true)
     |> assign(:releases_empty?, true)
     |> assign(:attached_instances_empty?, true)
     |> assign(:tree, empty_tree())
     |> assign(:current_path, nil)
     |> assign(:file, nil)
     |> assign(:file_error, nil)}
  end

  @impl true
  def handle_params(%{"repository_id" => repository_id} = params, uri, socket) do
    identity = socket.assigns.current_identity

    with {:ok, repository} <- Store.get_repository(identity, repository_id),
         {:ok, branches} <- RepositoryBrowser.branches(repository_root(), repository_id),
         {:ok, selected_branch} <- select_branch(repository, branches, params["ref"]),
         {:ok, browser_assigns} <-
           load_tab(
             socket.assigns.live_action,
             repository_id,
             selected_branch,
             params,
             identity
           ) do
      socket =
        socket
        |> assign(:current_repository_path, local_path(uri))
        |> assign(:page_title, page_title(repository, socket.assigns.live_action))
        |> assign(:repository, repository)
        |> assign(:selected_branch, selected_branch)
        |> assign(:branch_options, Enum.map(branches, & &1.name))
        |> assign(
          :branch_form,
          to_form(%{"branch" => selected_branch && selected_branch.name}, as: :browse)
        )
        |> assign(:branches_empty?, branches == [])
        |> stream(:branches, branches, reset: true)
        |> apply_browser_assigns(browser_assigns)

      subscribe_after_authorization(socket)
      {:noreply, socket}
    else
      {:error, _reason} ->
        {:noreply,
         socket
         |> put_flash(:error, "Repository not found or access was revoked.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  @impl true
  def handle_info(:repository_wakeup, socket) do
    repository_id = socket.assigns.repository["id"]

    case Store.get_repository(socket.assigns.current_identity, repository_id) do
      {:ok, _repository} ->
        {:noreply, push_patch(socket, to: socket.assigns.current_repository_path)}

      {:error, _reason} ->
        {:noreply,
         socket
         |> put_flash(:error, "Your repository access was revoked.")
         |> push_navigate(to: ~p"/organizations")}
    end
  end

  @impl true
  def handle_event("select-branch", %{"browse" => %{"branch" => branch}}, socket) do
    destination =
      case socket.assigns.live_action do
        :commits ->
          ~p"/repositories/#{socket.assigns.repository["id"]}/commits?#{[ref: branch]}"

        _files ->
          ~p"/repositories/#{socket.assigns.repository["id"]}/files?#{[ref: branch]}"
      end

    {:noreply, push_patch(socket, to: destination)}
  end

  @impl true
  def render(assigns) do
    ~H"""
    <Layouts.app flash={@flash} current_identity={@current_identity}>
      <%= if @repository do %>
        <.breadcrumbs id="repository-breadcrumbs">
          <:item navigate={~p"/organizations"}>Organizations</:item>
          <:item navigate={~p"/organizations/#{@repository["organization_id"]}"}>
            {@repository["organization_name"]}
          </:item>
          <:item navigate={~p"/projects/#{@repository["project_id"]}"}>
            {@repository["project_name"]}
          </:item>
          <:current>{@repository["name"]}</:current>
        </.breadcrumbs>

        <section class="repo-hero">
          <div class="repo-symbol">⌘</div>
          <div>
            <p class="eyebrow">Git repository</p>
            <h1>{@repository["name"]}</h1>
            <p class="mono">{friendly_ref(@repository["default_branch"])}</p>
          </div>
          <div class="repo-badges">
            <.tag tone={if(@repository["is_public"], do: "success", else: "neutral")}>
              {if @repository["is_public"], do: "public", else: "private"}
            </.tag>
            <.tag tone="success" dot>live</.tag>
          </div>
        </section>

        <.repository_tabs
          repository_id={@repository["id"]}
          active={@live_action}
          branch={@selected_branch && @selected_branch.name}
        />

        <section :if={@live_action == :commits} class="branch-toolbar">
          <.form for={@branch_form} id="branch-selector" phx-change="select-branch">
            <.input
              field={@branch_form[:branch]}
              type="select"
              label="Branch"
              options={@branch_options}
              disabled={@branches_empty?}
              class="branch-select"
            />
          </.form>
          <div :if={@selected_branch} class="branch-head">
            <span>Head</span>
            <code>{short_sha(@selected_branch.commit)}</code>
          </div>
        </section>

        <.files_view
          :if={@live_action == :files}
          repository={@repository}
          selected_branch={@selected_branch}
          tree={@tree}
          current_path={@current_path}
          file={@file}
          file_error={@file_error}
          branch_form={@branch_form}
          branch_options={@branch_options}
          branches_empty?={@branches_empty?}
        />
        <.commits_view
          :if={@live_action == :commits}
          streams={@streams}
          commits_empty?={@commits_empty?}
        />
        <.branches_view
          :if={@live_action == :branches}
          repository={@repository}
          streams={@streams}
          branches_empty?={@branches_empty?}
        />
        <.releases_view
          :if={@live_action == :releases}
          repository={@repository}
          streams={@streams}
          releases_empty?={@releases_empty?}
        />
        <.agents_view
          :if={@live_action == :agents}
          streams={@streams}
          attached_instances_empty?={@attached_instances_empty?}
        />
      <% end %>
    </Layouts.app>
    """
  end

  attr :repository, :map, required: true
  attr :selected_branch, :map, default: nil
  attr :tree, :map, required: true
  attr :current_path, :string, default: nil
  attr :file, :map, default: nil
  attr :file_error, :string, default: nil
  attr :branch_form, :map, required: true
  attr :branch_options, :list, required: true
  attr :branches_empty?, :boolean, required: true

  defp files_view(assigns) do
    ~H"""
    <section id="repository-files" class="repository-browser">
      <aside class="file-browser">
        <div class="file-branch-selector">
          <.form for={@branch_form} id="file-branch-selector" phx-change="select-branch">
            <.input
              field={@branch_form[:branch]}
              type="select"
              label="Branch"
              options={@branch_options}
              disabled={@branches_empty?}
              class="branch-select"
            />
          </.form>
          <code :if={@selected_branch}>{short_sha(@selected_branch.commit)}</code>
        </div>
        <div class="browser-heading">
          <span>Files</span>
          <.tag>{@tree.file_count}</.tag>
        </div>
        <.file_tree
          tree={@tree}
          repository_id={@repository["id"]}
          branch={@selected_branch && @selected_branch.name}
          current_path={@current_path}
        />
      </aside>
      <main class="file-viewer">
        <div :if={@file} class="file-heading">
          <div>
            <strong>{@file.entry.path}</strong>
            <small>{@file.language} · {format_bytes(@file.entry.size)}</small>
          </div>
          <code>{short_sha(@file.entry.object_id)}</code>
        </div>
        <pre :if={@file} id="file-contents"><code>{@file.contents}</code></pre>
        <div :if={@file_error} id="file-preview-error" class="viewer-empty">
          <.icon name="hero-document-minus" class="size-8" />
          <strong>Preview unavailable</strong>
          <p>{@file_error}</p>
        </div>
        <div :if={!@file && !@file_error} id="file-viewer-empty" class="viewer-empty">
          <.icon name="hero-cursor-arrow-rays" class="size-8" />
          <strong>Select a file</strong>
          <p>The tree is the exact snapshot at the selected branch head.</p>
        </div>
      </main>
    </section>
    """
  end

  attr :streams, :map, required: true
  attr :commits_empty?, :boolean, required: true

  defp commits_view(assigns) do
    ~H"""
    <section id="repository-commits" class="repository-list">
      <div class="list-heading">
        <span>Commit</span><span>Author</span><span>Date</span>
      </div>
      <div id="commits" phx-update="stream">
        <div :if={@commits_empty?} id="commits-empty" class="empty-state">
          No commits on this branch.
        </div>
        <article :for={{dom_id, commit} <- @streams.commits} id={dom_id} class="commit-row">
          <div class="commit-primary">
            <strong>{commit.subject}</strong>
            <code>{short_sha(commit.id)}</code>
          </div>
          <div class="commit-author">
            <strong>{commit.author_name}</strong>
            <small>{commit.author_email}</small>
          </div>
          <time datetime={commit.authored_at}>{display_time(commit.authored_at)}</time>
        </article>
      </div>
    </section>
    """
  end

  attr :repository, :map, required: true
  attr :streams, :map, required: true
  attr :branches_empty?, :boolean, required: true

  defp branches_view(assigns) do
    ~H"""
    <section id="repository-branches" class="repository-list">
      <div class="list-heading branch-list-heading">
        <span>Branch</span><span>Head commit</span><span>Updated</span>
      </div>
      <div id="branches" phx-update="stream">
        <div :if={@branches_empty?} id="branches-empty" class="empty-state">
          No branches have been pushed yet.
        </div>
        <article :for={{dom_id, branch} <- @streams.branches} id={dom_id} class="branch-row">
          <div class="branch-primary">
            <.link navigate={~p"/repositories/#{@repository["id"]}/files?#{[ref: branch.name]}"}>
              <.icon name="hero-code-bracket" class="size-4" />
              <strong>{branch.name}</strong>
            </.link>
            <.tag :if={branch.ref == @repository["default_branch"]} tone="accent">default</.tag>
          </div>
          <div>
            <code>{short_sha(branch.commit)}</code>
            <small>{branch.subject}</small>
          </div>
          <time datetime={branch.committed_at}>{display_time(branch.committed_at)}</time>
        </article>
      </div>
    </section>
    """
  end

  attr :repository, :map, required: true
  attr :streams, :map, required: true
  attr :releases_empty?, :boolean, required: true

  defp releases_view(assigns) do
    ~H"""
    <section id="repository-releases" class="repository-list">
      <div class="list-heading">
        <span>Release</span><span>Source</span><span>Artifacts</span>
      </div>
      <div id="releases" phx-update="stream">
        <div :if={@releases_empty?} id="releases-empty" class="empty-state">
          No immutable releases have been built from this repository.
        </div>
        <article :for={{dom_id, release} <- @streams.releases} id={dom_id} class="commit-row">
          <div class="commit-primary">
            <.link navigate={~p"/repositories/#{@repository["id"]}/releases/#{release["id"]}"}>
              <strong>{release["version"]}</strong>
            </.link>
            <.tag tone={release_tone(release["state"])}>{release["state"]}</.tag>
          </div>
          <div class="commit-author">
            <code>{short_sha(release["source_commit"])}</code>
            <small>{friendly_ref(release["source_ref"])}</small>
          </div>
          <div>
            <strong>{release["artifact_count"]} artifacts</strong>
            <small>{release["exported_agent_count"]} exported agents</small>
          </div>
        </article>
      </div>
    </section>
    """
  end

  attr :streams, :map, required: true
  attr :attached_instances_empty?, :boolean, required: true

  defp agents_view(assigns) do
    ~H"""
    <section id="repository-agents" class="repository-list">
      <div class="list-heading">
        <span>Project instance</span><span>Ref selector</span><span>Release</span>
      </div>
      <div id="attached-instances" phx-update="stream">
        <div :if={@attached_instances_empty?} id="attached-instances-empty" class="empty-state">
          No project agent instances are attached to this repository.
        </div>
        <article
          :for={{dom_id, attachment} <- @streams.attached_instances}
          id={dom_id}
          class="commit-row"
        >
          <div class="commit-primary">
            <.link navigate={
              ~p"/projects/#{attachment["project_id"]}/agents/#{attachment["instance_id"]}"
            }>
              <strong>{attachment["instance_name"]}</strong>
            </.link>
            <small>{attachment["project_name"]}</small>
          </div>
          <div class="commit-author">
            <code>{attachment["ref_selector"]}</code>
            <small>{attachment["trigger_policy"]}</small>
          </div>
          <div>
            <strong>{attachment["release_version"]}</strong>
            <small>{attachment["instance_state"]}</small>
          </div>
        </article>
      </div>
    </section>
    """
  end

  defp load_tab(action, _repository_id, nil, _params, _identity)
       when action in [:files, :commits] do
    {:ok,
     %{
       tree: empty_tree(),
       current_path: nil,
       file: nil,
       file_error: nil,
       commits: [],
       commits_empty?: true,
       releases: [],
       attached_instances: []
     }}
  end

  defp load_tab(:files, repository_id, selected_branch, params, _identity) do
    with {:ok, %{entries: entries}} <-
           RepositoryBrowser.tree(repository_root(), repository_id, selected_branch.name) do
      path = path_from_params(params)
      {file, file_error} = load_file(repository_id, selected_branch.name, path)

      {:ok,
       %{
         tree: build_tree(entries),
         current_path: path,
         file: file,
         file_error: file_error,
         commits: [],
         commits_empty?: true,
         releases: [],
         attached_instances: []
       }}
    end
  end

  defp load_tab(:commits, repository_id, selected_branch, _params, _identity) do
    with {:ok, %{commits: commits}} <-
           RepositoryBrowser.commits(repository_root(), repository_id, selected_branch.name) do
      {:ok,
       %{
         tree: empty_tree(),
         current_path: nil,
         file: nil,
         file_error: nil,
         commits: commits,
         commits_empty?: commits == [],
         releases: [],
         attached_instances: []
       }}
    end
  end

  defp load_tab(:branches, _repository_id, _selected_branch, _params, _identity) do
    {:ok,
     %{
       tree: empty_tree(),
       current_path: nil,
       file: nil,
       file_error: nil,
       commits: [],
       commits_empty?: true,
       releases: [],
       attached_instances: []
     }}
  end

  defp load_tab(:releases, repository_id, _selected_branch, _params, identity) do
    with {:ok, releases} <- Store.list_repository_releases(identity, repository_id) do
      {:ok, empty_browser_assigns() |> Map.put(:releases, releases)}
    end
  end

  defp load_tab(:agents, repository_id, _selected_branch, _params, identity) do
    with {:ok, instances} <- Store.list_repository_instances(identity, repository_id) do
      {:ok, empty_browser_assigns() |> Map.put(:attached_instances, instances)}
    end
  end

  defp apply_browser_assigns(socket, assigns) do
    socket
    |> assign(:tree, assigns.tree)
    |> assign(:current_path, assigns.current_path)
    |> assign(:file, assigns.file)
    |> assign(:file_error, assigns.file_error)
    |> assign(:commits_empty?, assigns.commits_empty?)
    |> assign(:releases_empty?, assigns.releases == [])
    |> assign(:attached_instances_empty?, assigns.attached_instances == [])
    |> stream(:commits, assigns.commits, reset: true)
    |> stream(:releases, assigns.releases, reset: true)
    |> stream(:attached_instances, assigns.attached_instances, reset: true)
  end

  defp empty_browser_assigns do
    %{
      tree: empty_tree(),
      current_path: nil,
      file: nil,
      file_error: nil,
      commits: [],
      commits_empty?: true,
      releases: [],
      attached_instances: []
    }
  end

  defp select_branch(_repository, [], _requested), do: {:ok, nil}

  defp select_branch(repository, branches, requested) do
    default = friendly_ref(repository["default_branch"])

    name =
      case requested do
        nil ->
          if Enum.any?(branches, &(&1.name == default)), do: default, else: hd(branches).name

        requested ->
          requested
      end

    case Enum.find(branches, &(&1.name == name)) do
      nil -> {:error, :branch_not_found}
      branch -> {:ok, branch}
    end
  end

  defp load_file(_repository_id, _branch, nil), do: {nil, nil}

  defp load_file(repository_id, branch, path) do
    case RepositoryBrowser.file(repository_root(), repository_id, branch, path) do
      {:ok, file} -> {file, nil}
      {:error, :binary_file} -> {nil, "Binary files are not rendered in the browser."}
      {:error, :file_too_large} -> {nil, "Files larger than 1 MiB are not rendered."}
      {:error, :not_found} -> {nil, "This path does not exist at the selected branch head."}
      {:error, _reason} -> {nil, "This file cannot be rendered safely."}
    end
  end

  defp path_from_params(%{"path" => segments}) when is_list(segments),
    do: Enum.join(segments, "/")

  defp path_from_params(_params), do: nil

  defp build_tree(entries) do
    entries
    |> Enum.reduce(empty_node("", ""), &insert_entry/2)
    |> finalize_tree(length(entries))
  end

  defp insert_entry(entry, tree) do
    components = String.split(entry.path, "/", trim: true)
    insert_components(tree, components, entry, [])
  end

  defp insert_components(tree, [name], entry, _parents) do
    file = Map.merge(entry, %{name: name})
    Map.update!(tree, :files, &[file | &1])
  end

  defp insert_components(tree, [directory | rest], entry, parents) do
    path = Enum.join(parents ++ [directory], "/")
    child = Map.get(tree.directories, directory, empty_node(directory, path))
    child = insert_components(child, rest, entry, parents ++ [directory])
    put_in(tree, [:directories, directory], child)
  end

  defp finalize_tree(tree, file_count \\ nil) do
    directories =
      tree.directories
      |> Map.values()
      |> Enum.map(&finalize_tree/1)
      |> Enum.sort_by(&String.downcase(&1.name))

    files = Enum.sort_by(tree.files, &String.downcase(&1.name))

    tree
    |> Map.put(:directories, directories)
    |> Map.put(:files, files)
    |> Map.put(:file_count, file_count || count_files(files, directories))
  end

  defp count_files(files, directories) do
    length(files) + Enum.sum(Enum.map(directories, & &1.file_count))
  end

  defp empty_tree, do: %{name: "", path: "", directories: [], files: [], file_count: 0}
  defp empty_node(name, path), do: %{name: name, path: path, directories: %{}, files: []}

  defp repository_root do
    Application.fetch_env!(:hephaestus_web, :repository_root)
  end

  defp subscribe_after_authorization(socket) do
    if connected?(socket), do: RunNotifier.subscribe_repositories()
  end

  defp local_path(uri) do
    parsed = URI.parse(uri)
    if parsed.query, do: "#{parsed.path}?#{parsed.query}", else: parsed.path
  end

  defp page_title(repository, :files), do: "#{repository["name"]} · Files"
  defp page_title(repository, :commits), do: "#{repository["name"]} · Commits"
  defp page_title(repository, :branches), do: "#{repository["name"]} · Branches"
  defp page_title(repository, :releases), do: "#{repository["name"]} · Releases"
  defp page_title(repository, :agents), do: "#{repository["name"]} · Agents"

  defp release_tone("published"), do: "success"
  defp release_tone("revoked"), do: "danger"
  defp release_tone(_state), do: "neutral"

  defp friendly_ref("refs/heads/" <> branch), do: branch
  defp friendly_ref(git_ref), do: git_ref

  defp short_sha(value), do: String.slice(value, 0, 10)

  defp display_time(value) do
    case DateTime.from_iso8601(value) do
      {:ok, date_time, _offset} -> Calendar.strftime(date_time, "%d %b %Y · %H:%M")
      _error -> value
    end
  end

  defp format_bytes(nil), do: "unknown size"
  defp format_bytes(bytes) when bytes < 1_024, do: "#{bytes} B"
  defp format_bytes(bytes), do: "#{Float.round(bytes / 1_024, 1)} KiB"

  defp tree_id(value) do
    :crypto.hash(:sha256, value)
    |> Base.url_encode64(padding: false)
    |> binary_part(0, 12)
  end
end
