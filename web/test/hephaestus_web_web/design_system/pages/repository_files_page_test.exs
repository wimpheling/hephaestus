defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryFilesPageTest do
  use ExUnit.Case, async: true
  import Phoenix.Component, only: [sigil_H: 2]
  import Phoenix.LiveViewTest
  alias HephaestusWebWeb.DesignSystem.Composites.RepositoryBrowser
  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryFilesPage
  alias HephaestusWebWeb.RepositoryPageFixtures

  @covered_states [:loading, :error, :reconnecting, :ready]
  @status_visual_states %{
    initial: :loading,
    loading: :loading,
    ready: :ready,
    submitting: :loading,
    error: :error,
    stale: :reconnecting,
    reconnecting: :reconnecting,
    access_revoked: :error
  }

  test "renders every lifecycle visual and the files route" do
    model = RepositoryPageFixtures.model()
    form = Phoenix.Component.to_form(model.browse_form, as: :browse)

    for state <- Map.values(@status_visual_states) do
      html =
        render_component(&RepositoryFilesPage.repository_files/1, %{
          state: state,
          model: model,
          branch_form: form,
          select_branch_event: "select-branch"
        })

      assert html =~ "repository-page-state" or html =~ "repository-files" or
               html =~ "repository-empty-push"
    end

    assert MapSet.new(Map.values(@status_visual_states)) == MapSet.new(@covered_states)
  end

  test "renders credential-free first-push instructions for an empty repository" do
    model =
      RepositoryPageFixtures.model()
      |> Map.put(:remote_url, "https://forge.example/repository-1")
      |> Map.put(:clone_command, "git clone https://heph-pat@forge.example/repository-1 source")
      |> Map.put(:default_branch, "trunk")

    html =
      render_component(&RepositoryFilesPage.repository_files/1, %{
        state: :ready,
        model: model,
        branch_form: Phoenix.Component.to_form(model.browse_form, as: :browse),
        select_branch_event: "select-branch"
      })

    assert html =~ "Push your first commit"
    assert html =~ "https://forge.example/repository-1"
    assert html =~ "git clone https://heph-pat@forge.example/repository-1 source"
    assert html =~ "Copy clone command"
    assert html =~ "hephaestus:copy-to-clipboard"
    assert html =~ "push -u origin trunk"
    assert html =~ "HEPHAESTUS_GIT_TOKEN"
    assert html =~ "agent.toml"
    refute html =~ "Bearer ey"
  end

  test "keeps first-push instructions visible while the watch refreshes" do
    model =
      RepositoryPageFixtures.model()
      |> Map.put(:remote_url, "https://forge.example/repository-1")
      |> Map.put(:default_branch, "trunk")

    for state <- [:loading, :reconnecting] do
      html =
        render_component(&RepositoryFilesPage.repository_files/1, %{
          state: state,
          model: model,
          branch_form: Phoenix.Component.to_form(model.browse_form, as: :browse),
          select_branch_event: "select-branch"
        })

      assert html =~ "repository-empty-push"
      refute html =~ "Repository unavailable"
    end
  end

  test "renders file contents as numbered source without template indentation" do
    model =
      RepositoryPageFixtures.model()
      |> Map.merge(%{
        branches_empty?: false,
        branch_options: ["main"],
        selected_branch: %{name: "main", commit: "0123456789abcdef"},
        tree: %{name: "", path: "", directories: [], files: [], file_count: 1},
        file: %{
          entry: %{path: "src/main.rs", size: 25, object_id: "0123456789abcdef"},
          language: "Rust",
          contents: "fn main() {\n    println!(\"hi\");\n}"
        }
      })

    html =
      render_component(&RepositoryFilesPage.repository_files/1, %{
        state: :ready,
        model: model,
        branch_form: Phoenix.Component.to_form(model.browse_form, as: :browse),
        select_branch_event: "select-branch"
      })

    assert html =~ ~s(id="file-contents-0123456789abcdef")
    assert html =~ ~s(phx-hook="SourceHighlight")
    assert html =~ ~s(data-source-language="Rust")
    assert html =~ ~s(class="file-source-number")
    assert html =~ ~s|<span class="file-source-content">fn main() {</span>|
    assert html =~ ~s|<span class="file-source-content">    println!(&quot;hi&quot;);</span>|
    refute html =~ ~s|<span class="file-source-content">\n|
  end

  test "groups repository navigation and tree in the left pane" do
    html = render_component(&repository_browser_fixture/1, %{})

    assert html =~ ~s(class="file-browser")
    assert html =~ ~s(aria-label="Repository files")
    assert html =~ ~s(class="file-viewer")
    assert html =~ ~s(aria-label="File preview")
    assert offset_of(html, "branch selector") < offset_of(html, "file tree")
    assert offset_of(html, "file tree") < offset_of(html, "file contents")
  end

  defp repository_browser_fixture(assigns) do
    _ = assigns

    ~H"""
    <RepositoryBrowser.repository_browser id="repository-browser-test">
      <:navigation>branch selector</:navigation>
      <:tree>file tree</:tree>
      <:content>file contents</:content>
    </RepositoryBrowser.repository_browser>
    """
  end

  defp offset_of(html, value), do: html |> :binary.match(value) |> elem(0)
end
