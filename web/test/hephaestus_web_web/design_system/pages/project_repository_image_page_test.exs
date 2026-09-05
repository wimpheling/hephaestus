defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectRepositoryImagePageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest, only: [render_component: 2]

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectRepositoryImagePage
  alias HephaestusWebWeb.ProjectRepositoryImageState

  @covered_states [:loading, :error, :reconnecting, :ready]

  @covered_statuses [
    :initial,
    :loading,
    :ready,
    :submitting,
    :error,
    :stale,
    :reconnecting,
    :access_revoked
  ]

  test "renders bounded redacted preparation history" do
    assert length(@covered_states) == 4
    assert @covered_statuses == ProjectRepositoryImageState.statuses()

    for status <- @covered_statuses do
      state = %{
        ProjectRepositoryImageState.new(%{project_id: "project-1", image_id: "image-1"})
        | status: status,
          data: %{
            project_id: "project-1",
            image_id: "image-1",
            image: image(),
            history: history()
          }
      }

      assert render(ProjectRepositoryImageState.present(state).status) != ""
    end

    html = render(:ready)

    assert html =~ "Preparation history"
    assert html =~ "sha256:def"
    refute html =~ "/private/"
  end

  defp render(state) do
    render_component(&ProjectRepositoryImagePage.project_repository_image_page/1,
      state: state,
      project_id: "project-1",
      image: image(),
      history: history()
    )
  end

  defp image,
    do: %{
      "display_name" => "Cooking blog",
      "source_revision" => "abc123",
      "base_image_reference" => "registry.example/base@sha256:abc",
      "status" => "ready",
      "image_reference" => "registry.example/blog@sha256:def"
    }

  defp history,
    do: [
      %{
        "phase" => "ready",
        "outcome" => "succeeded",
        "output_digest" => "sha256:def"
      }
    ]
end
