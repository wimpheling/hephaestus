defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectRepositoryImagesPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectRepositoryImagesPage
  alias HephaestusWebWeb.ProjectRepositoryImagesState

  @covered_states [:loading, :empty, :error, :reconnecting, :ready]

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

  test "renders repository image resources for every lifecycle status" do
    assert @covered_statuses == ProjectRepositoryImagesState.statuses()

    for status <- @covered_statuses do
      state = %{
        ProjectRepositoryImagesState.new(%{project_id: "project-1"})
        | status: status,
          data: %{project_id: "project-1", images: [image()]}
      }

      presentation = ProjectRepositoryImagesState.present(state)

      assert render_component(&ProjectRepositoryImagesPage.project_repository_images_page/1,
               state: presentation.status,
               project_id: "project-1",
               images: presentation.images,
               image_destination: fn image_id -> "/projects/project-1/images/#{image_id}" end
             ) != ""
    end

    assert length(@covered_states) == 5
  end

  defp image,
    do: %{
      "id" => "image-1",
      "display_name" => "Blog image",
      "key" => "blog",
      "source_revision" => "abc123",
      "status" => "ready"
    }
end
