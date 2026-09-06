defmodule HephaestusWebWeb.DesignSystem.Pages.RepositoryCommitDetailPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.RepositoryCommitDetailPage
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

  test "renders every lifecycle visual and an escaped bounded diff" do
    model = model()

    for state <- Map.values(@status_visual_states) do
      html =
        render_component(&RepositoryCommitDetailPage.repository_commit_detail/1, %{
          state: state,
          model: model
        })

      assert html =~ "repository-page-state" or html =~ "repository-commit-detail"
    end

    html =
      render_component(&RepositoryCommitDetailPage.repository_commit_detail/1, %{
        state: :ready,
        model: model
      })

    assert MapSet.new(Map.values(@status_visual_states)) == MapSet.new(@covered_states)
    assert html =~ "Add recipe"
    assert html =~ "&lt;script&gt;"
    refute html =~ "<script>"
    assert html =~ ~s(phx-hook="SourceHighlight")
    assert html =~ "diff-added"
    assert html =~ "commit-file-"
  end

  test "makes an unavailable commit actionable without exposing a diff" do
    html =
      render_component(&RepositoryCommitDetailPage.repository_commit_detail/1, %{
        state: :ready,
        model: Map.put(model(), :commit_detail, nil)
      })

    assert html =~ "Commit unavailable"
    refute html =~ "commit-file-"
  end

  defp model do
    RepositoryPageFixtures.model()
    |> Map.merge(%{
      selected_branch: %{name: "main", commit: "0123456789abcdef"},
      commit_detail: %{
        "commit" => %{
          "id" => "0123456789abcdef",
          "subject" => "Add recipe",
          "author_name" => "Ada"
        },
        "body" => "Document the dinner recipe.",
        "selected_parent" => "fedcba9876543210",
        "truncated" => false,
        "files" => [
          %{
            "path" => "recipes/dinner.md",
            "previous_path" => "",
            "additions" => 1,
            "deletions" => 0,
            "binary" => false,
            "state" => "DIFF_FILE_STATE_AVAILABLE",
            "hunks" => [
              %{
                "old_start" => 0,
                "old_lines" => 0,
                "new_start" => 1,
                "new_lines" => 1,
                "lines" => [
                  %{
                    "kind" => "DIFF_LINE_KIND_ADDED",
                    "old_line" => nil,
                    "new_line" => 1,
                    "text" => "<script>escaped</script>"
                  }
                ]
              }
            ]
          }
        ]
      }
    })
  end
end
