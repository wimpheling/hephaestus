defmodule HephaestusWeb.RepositoryBrowserTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.RepositoryBrowser

  @repository_id "018f689a-a81d-7c2e-943f-3a41f7981234"

  setup do
    root =
      Path.join(
        System.tmp_dir!(),
        "hephaestus-repository-browser-#{System.unique_integer([:positive])}"
      )

    work = Path.join(root, "work")
    repositories = Path.join(root, "repositories")
    bare = Path.join(repositories, "#{@repository_id}.git")

    File.mkdir_p!(repositories)
    git!(["init", "--initial-branch=main", work])
    git!(["-C", work, "config", "user.name", "Ada Agent"])
    git!(["-C", work, "config", "user.email", "ada@example.test"])

    File.mkdir_p!(Path.join(work, "lib"))
    File.write!(Path.join(work, "README.md"), "# Exact tree\n")
    File.write!(Path.join(work, "lib/agent.ex"), "defmodule Agent do\nend\n")
    git!(["-C", work, "add", "."])
    git!(["-C", work, "commit", "-m", "initial tree"])
    {main_commit, 0} = System.cmd("git", ["-C", work, "rev-parse", "HEAD"])

    git!(["-C", work, "checkout", "-b", "feature/review"])
    File.write!(Path.join(work, "feature.txt"), "branch content\n")
    git!(["-C", work, "add", "feature.txt"])
    git!(["-C", work, "commit", "-m", "add feature"])
    git!(["clone", "--bare", work, bare])

    on_exit(fn -> File.rm_rf!(root) end)

    %{
      repositories: repositories,
      bare: bare,
      main_commit: String.trim(main_commit)
    }
  end

  test "lists branch heads and exact commit history", context do
    assert {:ok, branches} = RepositoryBrowser.branches(context.repositories, @repository_id)
    assert Enum.map(branches, & &1.name) == ["feature/review", "main"]

    assert {:ok, %{branch: branch, commits: commits}} =
             RepositoryBrowser.commits(context.repositories, @repository_id, "main")

    assert branch.commit == context.main_commit
    assert [%{id: id, subject: "initial tree", author_name: "Ada Agent"}] = commits
    assert id == context.main_commit
  end

  test "reads a file directly from the selected branch tree", context do
    assert {:ok, %{branch: %{commit: commit}, entries: entries}} =
             RepositoryBrowser.tree(context.repositories, @repository_id, "main")

    assert commit == context.main_commit
    assert Enum.map(entries, & &1.path) == ["README.md", "lib/agent.ex"]

    assert {:ok, file} =
             RepositoryBrowser.file(
               context.repositories,
               @repository_id,
               "main",
               "lib/agent.ex"
             )

    assert file.contents == "defmodule Agent do\nend\n"
    assert file.language == "elixir"

    assert {:error, :not_found} =
             RepositoryBrowser.file(
               context.repositories,
               @repository_id,
               "main",
               "feature.txt"
             )
  end

  test "rejects invalid identifiers, path traversal, symlinks, and unknown branches", context do
    assert {:error, :invalid_repository_id} =
             RepositoryBrowser.repository_path(context.repositories, "../../etc")

    assert {:error, :invalid_path} =
             RepositoryBrowser.file(
               context.repositories,
               @repository_id,
               "main",
               "../config"
             )

    assert {:error, :branch_not_found} =
             RepositoryBrowser.tree(context.repositories, @repository_id, "--all")

    symlink_id = "018f689a-a81d-7c2e-943f-3a41f7985678"
    File.ln_s!(context.bare, Path.join(context.repositories, "#{symlink_id}.git"))

    assert {:error, :repository_symlink} =
             RepositoryBrowser.repository_path(context.repositories, symlink_id)
  end

  defp git!(arguments) do
    case System.cmd("git", arguments, stderr_to_stdout: true) do
      {_output, 0} -> :ok
      {output, status} -> flunk("git #{inspect(arguments)} failed (#{status}): #{output}")
    end
  end
end
