defmodule HephaestusWebWeb.RepositoryRouteModel do
  @moduledoc false

  alias HephaestusWeb.RPC.{Client, Error, ProductEvents}
  alias HephaestusWebWeb.ProductEventReducer

  @statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]

  def statuses, do: @statuses

  def begin_watch(state), do: ProductEventReducer.begin_watch(state)
  def watch_scope(state), do: {:repository, state.data.repository_id}

  def watch(identity, state, owner, generation) do
    ProductEvents.watch(
      identity,
      watch_scope(state),
      ProductEventReducer.committed_cursor(state.cursor),
      &deliver_watch(&1, owner, generation)
    )
  end

  def new(module, repository_id) do
    struct(module,
      data: %{
        repository_id: repository_id,
        repository: nil,
        selected_branch: nil,
        branch_options: [],
        branches: [],
        commits: [],
        builds: [],
        releases: [],
        attached_instances: [],
        tree: empty_tree(),
        current_path: nil,
        file: nil,
        file_error: nil,
        params: %{},
        uri: nil,
        remote_url: nil
      },
      form: %{browse: %{"branch" => ""}}
    )
  end

  def reduce(state, {:load, params, uri}, action) do
    # The first route load shares generation 1 with the watch started at mount;
    # later parameter loads keep that active watch generation intact.
    generation = max(state.stream_generation, 1)
    status = if state.data.repository, do: :stale, else: :loading

    {%{state | status: status, error: nil, stream_generation: generation},
     [{:load, generation, action, state.data.repository_id, params, uri}]}
  end

  def reduce(state, :refresh, action) do
    reduce(state, {:load, state.data.params, state.data.uri}, action)
  end

  def reduce(state, {:request_build, attributes}, :builds) do
    generation = state.stream_generation + 1

    {%{
       state
       | status: :submitting,
         error: nil,
         stream_generation: generation,
         form: Map.put(state.form, :build, attributes)
     }, [{:request_build, state.data.repository_id, attributes}]}
  end

  def reduce(state, {:request_build_result, {:ok, %{"build_id" => build_id}}}, :builds) do
    destination = "/repositories/#{state.data.repository_id}/builds/#{build_id}"
    {%{state | status: :ready, error: nil}, [{:navigate, destination}]}
  end

  def reduce(state, {:request_build_result, {:ok, _receipt}}, :builds),
    do: reduce(state, :refresh, :builds)

  def reduce(state, {:request_build_result, {:error, _reason}}, :builds) do
    message = "The build could not be requested."
    {%{state | status: :error, error: message}, [{:flash, :error, message}]}
  end

  def reduce(state, :disconnected, _action), do: {%{state | status: :reconnecting}, []}

  def reduce(%{data: %{repository: repository}} = state, :connected, action)
      when not is_nil(repository),
      do: reduce(state, :refresh, action)

  def reduce(state, :connected, _action), do: {%{state | status: :loading}, []}

  def reduce(state, {:watch, response}, action) do
    ProductEventReducer.reduce(state, response, relevant_events(action))
  end

  def reduce(state, :watch_ended, _action), do: ProductEventReducer.reconnect(state)

  def reduce(state, {:select_branch, branch}, action) when action in [:files, :commits] do
    repository_id = state.data.repository_id

    destination =
      case action do
        :commits -> "/repositories/#{repository_id}/commits?ref=#{URI.encode_www_form(branch)}"
        :files -> "/repositories/#{repository_id}/files?ref=#{URI.encode_www_form(branch)}"
      end

    {state, [{:patch, destination}]}
  end

  def reduce(
        %{stream_generation: generation} = state,
        {
          :loaded,
          generation,
          {:ok, data}
        },
        _action
      ) do
    form = %{browse: %{"branch" => data.selected_branch && data.selected_branch.name}}

    state = %{
      state
      | status: :ready,
        data: Map.merge(state.data, data),
        form: form,
        error: nil
    }

    ProductEventReducer.snapshot_complete(state)
  end

  def reduce(
        %{stream_generation: generation} = state,
        {
          :loaded,
          generation,
          {:error, _reason}
        },
        _action
      ) do
    message = "Repository not found or access was revoked."

    {%{state | status: :access_revoked, error: message},
     [{:flash, :error, message}, {:navigate, "/organizations"}]}
  end

  def reduce(state, {:loaded, _stale_generation, _result}, _action), do: {state, []}

  def reduce(state, {:effect_failed, _reason}, _action) do
    message = "Repository data is temporarily unavailable."
    {%{state | status: :error, error: message}, [{:flash, :error, message}]}
  end

  def execute({:load, generation, action, repository_id, params, uri}, identity) do
    result =
      with {:ok, repository} <- Client.get_repository(identity, repository_id),
           {:ok, branches} <- branches_for(identity, repository_id, action),
           {:ok, selected_branch} <- select_branch(repository, branches, params["ref"]),
           {:ok, route_data} <-
             load_route(action, repository_id, selected_branch, params, identity) do
        {:ok,
         route_data
         |> Map.merge(%{
           repository_id: repository_id,
           repository: repository,
           remote_url: remote_url(uri, repository_id),
           selected_branch: selected_branch,
           branch_options: Enum.map(branches, & &1.name),
           branches: branches,
           params: params,
           uri: local_path(uri)
         })}
      end

    {:loaded, generation, result}
  end

  defp branches_for(identity, repository_id, action)
       when action in [:files, :commits, :branches],
       do: Client.branches(identity, repository_id)

  defp branches_for(_identity, _repository_id, _action), do: {:ok, []}

  def execute(state, identity, generation, action) do
    execute(
      {:load, generation, action, state.data.repository_id, state.data.params, state.data.uri},
      identity
    )
  end

  def present(state, action) do
    repository = state.data.repository

    %{
      state: page_state(state.status),
      repository: repository,
      selected_branch: state.data.selected_branch,
      branch_options: state.data.branch_options,
      browse_form: state.form.browse,
      build_request_form:
        Phoenix.Component.to_form(
          state.form[:build] ||
            %{
              "source_commit" => "",
              "build_definition_hash" => "",
              "configuration_hash" => ""
            },
          as: :build
        ),
      branches_empty?: state.data.branches == [],
      commits_empty?: state.data.commits == [],
      builds_empty?: state.data.builds == [],
      builds_unavailable?: false,
      releases_empty?: state.data.releases == [],
      attached_instances_empty?: state.data.attached_instances == [],
      tree: state.data.tree,
      current_path: state.data.current_path,
      file: state.data.file,
      file_error: state.data.file_error,
      branches: state.data.branches,
      commits: state.data.commits,
      builds: state.data.builds,
      releases: state.data.releases,
      attached_instances: state.data.attached_instances,
      remote_url: state.data.remote_url,
      default_branch: repository && friendly_ref(repository["default_branch"]),
      error: state.error,
      tabs: tabs(repository, state.data.selected_branch),
      destinations: destinations(repository),
      active: action
    }
  end

  defp load_route(action, _repository_id, nil, _params, _identity)
       when action in [:files, :commits] do
    {:ok, empty_route_data()}
  end

  defp load_route(:files, repository_id, selected_branch, params, identity) do
    with {:ok, %{entries: entries}} <-
           Client.tree(identity, repository_id, selected_branch.name) do
      path = path_from_params(params)
      {file, file_error} = load_file(identity, repository_id, selected_branch.name, path)

      {:ok,
       empty_route_data()
       |> Map.merge(%{
         tree: build_tree(entries),
         current_path: path,
         file: file,
         file_error: file_error
       })}
    end
  end

  defp load_route(:commits, repository_id, selected_branch, _params, identity) do
    with {:ok, %{commits: commits}} <-
           Client.commits(identity, repository_id, selected_branch.name) do
      {:ok, empty_route_data() |> Map.put(:commits, commits)}
    end
  end

  defp load_route(:builds, repository_id, _selected_branch, _params, identity) do
    with {:ok, builds} <- Client.list_builds(identity, repository_id) do
      {:ok, empty_route_data() |> Map.put(:builds, builds)}
    end
  end

  defp load_route(:branches, _repository_id, _selected_branch, _params, _identity),
    do: {:ok, empty_route_data()}

  defp load_route(:releases, repository_id, _selected_branch, _params, identity) do
    with {:ok, releases} <- Client.list_repository_releases(identity, repository_id) do
      {:ok, empty_route_data() |> Map.put(:releases, releases)}
    end
  end

  defp load_route(:agents, repository_id, _selected_branch, _params, identity) do
    with {:ok, instances} <- Client.list_repository_instances(identity, repository_id) do
      {:ok, empty_route_data() |> Map.put(:attached_instances, instances)}
    end
  end

  defp relevant_events(action) when action in [:files, :commits, :branches],
    do: [:repository_changed, :repository_ref_changed]

  defp relevant_events(:releases),
    do: [:repository_changed, :build_changed, :release_changed, :artifact_changed]

  defp relevant_events(:builds),
    do: [:repository_changed, :build_changed, :release_changed, :artifact_changed]

  defp relevant_events(:agents),
    do: [:repository_changed, :agent_instance_changed]

  defp deliver_watch(response, owner, generation) do
    send(owner, {:page_watch, generation, response})

    case response.item do
      {kind, _value} when kind in [:retention_gap, :access_revoked] -> :halt
      _item -> :cont
    end
  end

  defp empty_route_data do
    %{
      tree: empty_tree(),
      current_path: nil,
      file: nil,
      file_error: nil,
      commits: [],
      builds: [],
      releases: [],
      attached_instances: []
    }
  end

  defp select_branch(_repository, [], _requested), do: {:ok, nil}

  defp select_branch(repository, branches, requested) do
    default = friendly_ref(repository["default_branch"])

    name =
      case requested do
        nil -> if Enum.any?(branches, &(&1.name == default)), do: default, else: hd(branches).name
        requested -> requested
      end

    case Enum.find(branches, &(&1.name == name)) do
      nil -> {:error, :branch_not_found}
      branch -> {:ok, branch}
    end
  end

  defp load_file(_identity, _repository_id, _branch, nil), do: {nil, nil}

  defp load_file(identity, repository_id, branch, path) do
    case Client.file(identity, repository_id, branch, path) do
      {:ok, file} ->
        {file, nil}

      {:error, %Error{kind: :precondition}} ->
        {nil, "Binary files are not rendered in the browser."}

      {:error, %Error{kind: :size_limit}} ->
        {nil, "Files larger than 1 MiB are not rendered."}

      {:error, %Error{kind: :not_found}} ->
        {nil, "This path does not exist at the selected branch head."}

      {:error, _reason} ->
        {nil, "This file cannot be rendered safely."}
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

  defp tabs(nil, _selected_branch), do: []

  defp tabs(repository, selected_branch) do
    repository_id = repository["id"]
    ref = selected_branch && selected_branch.name
    suffix = if ref, do: "?ref=#{URI.encode_www_form(ref)}", else: ""

    [
      %{
        key: :files,
        label: "Files",
        icon: "hero-folder",
        destination: "/repositories/#{repository_id}/files#{suffix}"
      },
      %{
        key: :commits,
        label: "Commits",
        icon: "hero-clock",
        destination: "/repositories/#{repository_id}/commits#{suffix}"
      },
      %{
        key: :branches,
        label: "Branches",
        icon: "hero-code-bracket",
        destination: "/repositories/#{repository_id}/branches"
      },
      %{
        key: :builds,
        label: "Builds",
        icon: "hero-cpu-chip",
        destination: "/repositories/#{repository_id}/builds"
      },
      %{
        key: :releases,
        label: "Releases",
        icon: "hero-cube-transparent",
        destination: "/repositories/#{repository_id}/releases"
      },
      %{
        key: :agents,
        label: "Agents",
        icon: "hero-cpu-chip",
        destination: "/repositories/#{repository_id}/agents"
      }
    ]
  end

  defp destinations(nil), do: %{}

  defp destinations(repository) do
    %{
      organization_index: "/organizations",
      organization: "/organizations/#{repository["organization_id"]}",
      project: "/projects/#{repository["project_id"]}"
    }
  end

  defp page_state(status) when status in [:initial, :loading, :submitting], do: :loading
  defp page_state(:ready), do: :ready
  defp page_state(status) when status in [:stale, :reconnecting], do: :reconnecting
  defp page_state(_status), do: :error

  defp local_path(nil), do: nil

  defp local_path(uri) when is_binary(uri) do
    parsed = URI.parse(uri)
    if parsed.query, do: "#{parsed.path}?#{parsed.query}", else: parsed.path
  end

  # Git credentials must never be embedded in this URL. Keep only the public
  # request origin and the canonical repository UUID route component.
  defp remote_url(nil, repository_id), do: "/#{repository_id}"

  defp remote_url(uri, repository_id) when is_binary(uri) do
    parsed = URI.parse(uri)

    case {parsed.scheme, parsed.host} do
      {scheme, host} when is_binary(scheme) and is_binary(host) ->
        URI.to_string(%URI{
          scheme: scheme,
          host: host,
          port: parsed.port,
          path: "/#{repository_id}"
        })

      _missing_origin ->
        "/#{repository_id}"
    end
  end

  defp friendly_ref("refs/heads/" <> branch), do: branch
  defp friendly_ref(git_ref), do: git_ref

  defp empty_tree, do: %{name: "", path: "", directories: [], files: [], file_count: 0}
  defp empty_node(name, path), do: %{name: name, path: path, directories: %{}, files: []}
end
