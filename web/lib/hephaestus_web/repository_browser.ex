defmodule HephaestusWeb.RepositoryBrowser do
  @moduledoc """
  Read-only access to the canonical bare Git repository layout.

  Repository locations are derived solely from validated opaque identifiers.
  Git is invoked directly with an argument vector; this module never creates a
  checkout or evaluates repository-controlled shell content.
  """

  @branch_prefix "refs/heads/"
  @max_commits 100
  @max_blob_bytes 1_048_576

  @type branch :: %{
          name: String.t(),
          ref: String.t(),
          commit: String.t(),
          committed_at: String.t(),
          subject: String.t()
        }

  @type commit :: %{
          id: String.t(),
          parents: [String.t()],
          author_name: String.t(),
          author_email: String.t(),
          authored_at: String.t(),
          subject: String.t()
        }

  @type tree_entry :: %{
          mode: String.t(),
          type: String.t(),
          object_id: String.t(),
          size: non_neg_integer() | nil,
          path: String.t()
        }

  @spec branches(Path.t(), Ecto.UUID.t()) :: {:ok, [branch()]} | {:error, term()}
  def branches(root, repository_id) do
    with {:ok, repository} <- repository_path(root, repository_id),
         {:ok, output} <-
           git(repository, [
             "for-each-ref",
             "--sort=refname",
             "--format=%(refname)%00%(objectname)%00%(committerdate:iso-strict)%00%(subject)",
             "refs/heads/"
           ]) do
      parse_branches(output)
    end
  end

  @spec commits(Path.t(), Ecto.UUID.t(), String.t()) ::
          {:ok, %{branch: branch(), commits: [commit()]}} | {:error, term()}
  def commits(root, repository_id, branch_name) do
    with {:ok, repository} <- repository_path(root, repository_id),
         {:ok, branch} <- resolve_branch(repository, branch_name),
         {:ok, output} <-
           git(repository, [
             "log",
             "-z",
             "--max-count=#{@max_commits}",
             "--format=%H%x00%P%x00%an%x00%ae%x00%aI%x00%s",
             branch.commit
           ]),
         {:ok, commits} <- parse_commits(output) do
      {:ok, %{branch: branch, commits: commits}}
    end
  end

  @spec tree(Path.t(), Ecto.UUID.t(), String.t()) ::
          {:ok, %{branch: branch(), entries: [tree_entry()]}} | {:error, term()}
  def tree(root, repository_id, branch_name) do
    with {:ok, repository} <- repository_path(root, repository_id),
         {:ok, branch} <- resolve_branch(repository, branch_name),
         {:ok, output} <-
           git(repository, ["ls-tree", "-r", "-z", "-l", "--full-tree", branch.commit]),
         {:ok, entries} <- parse_tree(output) do
      {:ok, %{branch: branch, entries: entries}}
    end
  end

  @spec file(Path.t(), Ecto.UUID.t(), String.t(), String.t()) ::
          {:ok, %{entry: tree_entry(), contents: String.t(), language: String.t()}}
          | {:error, term()}
  def file(root, repository_id, branch_name, path) do
    with :ok <- validate_git_path(path),
         {:ok, repository} <- repository_path(root, repository_id),
         {:ok, branch} <- resolve_branch(repository, branch_name),
         {:ok, output} <-
           git(repository, ["ls-tree", "-r", "-z", "-l", "--full-tree", branch.commit]),
         {:ok, entries} <- parse_tree(output),
         %{} = entry <- Enum.find(entries, &(&1.path == path)),
         :ok <- validate_blob(entry),
         {:ok, contents} <- git(repository, ["cat-file", "blob", entry.object_id]),
         :ok <- validate_text(contents) do
      {:ok, %{entry: entry, contents: contents, language: language(path)}}
    else
      nil -> {:error, :not_found}
      {:error, _reason} = error -> error
    end
  end

  @doc false
  @spec repository_path(Path.t(), String.t()) :: {:ok, Path.t()} | {:error, term()}
  def repository_path(root, repository_id) do
    with {:ok, canonical_id} <- Ecto.UUID.cast(repository_id),
         expanded_root <- Path.expand(root),
         candidate <- Path.join(expanded_root, "#{canonical_id}.git"),
         true <- Path.dirname(candidate) == expanded_root,
         {:ok, %File.Stat{type: :directory}} <- File.lstat(candidate),
         {:ok, %File.Stat{type: :directory}} <- File.stat(candidate) do
      {:ok, candidate}
    else
      :error -> {:error, :invalid_repository_id}
      false -> {:error, :invalid_repository_path}
      {:ok, %File.Stat{type: :symlink}} -> {:error, :repository_symlink}
      {:ok, _stat} -> {:error, :invalid_repository}
      {:error, :enoent} -> {:error, :not_found}
      {:error, reason} -> {:error, {:filesystem, reason}}
    end
  end

  defp resolve_branch(repository, requested_name) do
    with {:ok, output} <-
           git(repository, [
             "for-each-ref",
             "--sort=refname",
             "--format=%(refname)%00%(objectname)%00%(committerdate:iso-strict)%00%(subject)",
             "refs/heads/"
           ]),
         {:ok, branches} <- parse_branches(output) do
      case Enum.find(branches, &(&1.name == requested_name)) do
        nil -> {:error, :branch_not_found}
        branch -> {:ok, branch}
      end
    end
  end

  defp parse_branches(output) do
    output
    |> String.split("\n", trim: true)
    |> Enum.reduce_while({:ok, []}, fn record, {:ok, branches} ->
      case String.split(record, <<0>>, parts: 4) do
        [@branch_prefix <> name = git_ref, commit, committed_at, subject]
        when name != "" ->
          branch = %{
            name: name,
            ref: git_ref,
            commit: commit,
            committed_at: committed_at,
            subject: subject
          }

          {:cont, {:ok, [branch | branches]}}

        _ ->
          {:halt, {:error, :invalid_git_output}}
      end
    end)
    |> reverse_result()
  end

  defp parse_commits(output) do
    fields =
      output
      |> :binary.split(<<0>>, [:global])
      |> drop_trailing_empty()

    fields
    |> Enum.chunk_every(6)
    |> Enum.reduce_while({:ok, []}, fn
      [id, parents, author_name, author_email, authored_at, subject], {:ok, commits} ->
        commit = %{
          id: id,
          parents: String.split(parents, " ", trim: true),
          author_name: author_name,
          author_email: author_email,
          authored_at: authored_at,
          subject: subject
        }

        {:cont, {:ok, [commit | commits]}}

      _fields, _accumulator ->
        {:halt, {:error, :invalid_git_output}}
    end)
    |> reverse_result()
  end

  defp drop_trailing_empty(fields) do
    case Enum.reverse(fields) do
      ["" | rest] -> Enum.reverse(rest)
      _fields -> fields
    end
  end

  defp parse_tree(output) do
    output
    |> :binary.split(<<0>>, [:global, :trim_all])
    |> Enum.reduce_while({:ok, []}, fn record, {:ok, entries} ->
      with [metadata, path] <- :binary.split(record, "\t"),
           true <- String.valid?(path),
           [mode, type, object_id, size] <- String.split(metadata, " ", trim: true),
           {:ok, parsed_size} <- parse_size(size) do
        entry = %{
          mode: mode,
          type: type,
          object_id: object_id,
          size: parsed_size,
          path: path
        }

        {:cont, {:ok, [entry | entries]}}
      else
        _reason -> {:halt, {:error, :invalid_git_output}}
      end
    end)
    |> reverse_result()
  end

  defp parse_size("-"), do: {:ok, nil}

  defp parse_size(value) do
    case Integer.parse(value) do
      {size, ""} when size >= 0 -> {:ok, size}
      _other -> {:error, :invalid_size}
    end
  end

  defp reverse_result({:ok, values}), do: {:ok, Enum.reverse(values)}
  defp reverse_result({:error, _reason} = error), do: error

  defp validate_git_path(path) when is_binary(path) do
    components = String.split(path, "/", trim: false)

    if String.valid?(path) and path != "" and
         Enum.all?(components, &valid_path_component?/1) do
      :ok
    else
      {:error, :invalid_path}
    end
  end

  defp validate_git_path(_path), do: {:error, :invalid_path}

  defp valid_path_component?(component) do
    component not in ["", ".", ".."] and
      not String.contains?(component, [<<0>>, "\\"])
  end

  defp validate_blob(%{type: "blob", size: size})
       when is_integer(size) and size <= @max_blob_bytes,
       do: :ok

  defp validate_blob(%{type: "blob"}), do: {:error, :file_too_large}
  defp validate_blob(_entry), do: {:error, :not_a_blob}

  defp validate_text(contents) do
    if String.valid?(contents) and not String.contains?(contents, <<0>>) do
      :ok
    else
      {:error, :binary_file}
    end
  end

  defp language(path) do
    case Path.extname(path) do
      ".ex" -> "elixir"
      ".exs" -> "elixir"
      ".rs" -> "rust"
      ".toml" -> "toml"
      ".md" -> "markdown"
      ".json" -> "json"
      ".sql" -> "sql"
      ".sh" -> "shell"
      ".yml" -> "yaml"
      ".yaml" -> "yaml"
      _extension -> "text"
    end
  end

  defp git(repository, arguments) do
    executable = System.find_executable("git")

    if executable do
      case System.cmd(
             executable,
             ["--git-dir", repository | arguments],
             env: [
               {"GIT_CONFIG_NOSYSTEM", "1"},
               {"GIT_OPTIONAL_LOCKS", "0"},
               {"GIT_PAGER", "cat"},
               {"GIT_TERMINAL_PROMPT", "0"},
               {"LC_ALL", "C.UTF-8"}
             ],
             stderr_to_stdout: true
           ) do
        {output, 0} -> {:ok, output}
        {_output, _status} -> {:error, :git_failed}
      end
    else
      {:error, :git_unavailable}
    end
  end
end
