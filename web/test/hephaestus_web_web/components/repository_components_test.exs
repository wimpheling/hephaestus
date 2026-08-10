defmodule HephaestusWebWeb.RepositoryComponentsTest do
  use ExUnit.Case, async: true

  use HephaestusWebWeb, :html

  import Phoenix.LiveViewTest

  @repository_id "018f689a-a81d-7c2e-943f-3a41f7981234"

  test "repository tabs are deep links and identify only the active tab" do
    html =
      render_component(&tabs_fixture/1,
        repository_id: @repository_id,
        active: :commits,
        branch: "feature/review"
      )

    document = LazyHTML.from_fragment(html)

    assert count(document, "#repository-tabs a") == 6

    assert count(document, ~s|#repository-tabs a[aria-current="page"]|) == 1

    assert count(
             document,
             ~s|a[href="/repositories/#{@repository_id}/commits?ref=feature%2Freview"]|
           ) == 1

    assert count(
             document,
             ~s|a[href="/repositories/#{@repository_id}/releases"]|
           ) == 1

    assert count(
             document,
             ~s|a[href="/repositories/#{@repository_id}/agents"]|
           ) == 1
  end

  test "file tree opens only the directory containing the selected file" do
    tree = %{
      name: "",
      path: "",
      file_count: 3,
      files: [%{name: "README.md", path: "README.md", mode: "100644"}],
      directories: [
        %{
          name: "lib",
          path: "lib",
          file_count: 1,
          directories: [],
          files: [%{name: "agent.ex", path: "lib/agent.ex", mode: "100644"}]
        },
        %{
          name: "test",
          path: "test",
          file_count: 1,
          directories: [],
          files: [%{name: "agent_test.exs", path: "test/agent_test.exs", mode: "100644"}]
        }
      ]
    }

    html =
      render_component(&tree_fixture/1,
        tree: tree,
        repository_id: @repository_id,
        branch: "main",
        current_path: "lib/agent.ex"
      )

    document = LazyHTML.from_fragment(html)

    assert count(document, "#repository-file-tree details") == 2
    assert count(document, "#repository-file-tree details[open]") == 1
    assert count(document, "#repository-file-tree details[open] summary span:last-child") == 1
    assert text(document, "#repository-file-tree details[open] summary span:last-child") == "lib"
    assert count(document, "#repository-file-tree .tree-file") == 3
    assert count(document, "#repository-file-tree .tree-file.active") == 1

    assert count(
             document,
             ~s|a[href="/repositories/#{@repository_id}/files/lib/agent.ex?ref=main"]|
           ) == 1
  end

  def tabs_fixture(assigns) do
    root = "/repositories/#{assigns.repository_id}"
    query = URI.encode_query(%{"ref" => assigns.branch})

    assigns =
      assign(assigns, :items, [
        %{key: :files, label: "Files", destination: "#{root}/files?#{query}"},
        %{key: :commits, label: "Commits", destination: "#{root}/commits?#{query}"},
        %{key: :branches, label: "Branches", destination: "#{root}/branches"},
        %{key: :builds, label: "Builds", destination: "#{root}/builds"},
        %{key: :releases, label: "Releases", destination: "#{root}/releases"},
        %{key: :agents, label: "Agents", destination: "#{root}/agents"}
      ])

    ~H"""
    <.tab_navigation id="repository-tabs" label="Repository" items={@items} active={@active} />
    """
  end

  def tree_fixture(assigns) do
    tree = decorate_tree(assigns.tree, assigns.repository_id, assigns.branch)
    assigns = assign(assigns, :tree, tree)

    ~H"""
    <.repository_tree tree={@tree} current_path={assigns[:current_path]} />
    """
  end

  defp decorate_tree(node, repository_id, branch) do
    %{
      node
      | directories: Enum.map(node.directories, &decorate_tree(&1, repository_id, branch)),
        files:
          Enum.map(node.files, fn file ->
            encoded_path =
              file.path
              |> String.split("/", trim: true)
              |> Enum.map_join("/", &URI.encode/1)

            query = URI.encode_query(%{"ref" => branch})
            destination = "/repositories/#{repository_id}/files/#{encoded_path}?#{query}"
            Map.put(file, :destination, destination)
          end)
    }
  end

  defp count(document, selector) do
    document
    |> LazyHTML.query(selector)
    |> LazyHTML.to_tree()
    |> length()
  end

  defp text(document, selector) do
    document
    |> LazyHTML.query(selector)
    |> LazyHTML.text()
  end
end
