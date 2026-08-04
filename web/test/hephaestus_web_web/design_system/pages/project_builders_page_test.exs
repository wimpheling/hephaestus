defmodule HephaestusWebWeb.DesignSystem.Pages.ProjectBuildersPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.ProjectBuildersPage
  alias HephaestusWebWeb.ProjectBuildersState

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

  test "renders every lifecycle state through its bounded visual state" do
    assert @covered_statuses == ProjectBuildersState.statuses()

    for status <- @covered_statuses do
      state = Map.fetch!(@status_visual_states, status)
      html = render_component(&ProjectBuildersPage.project_builders_page/1, assigns(state))

      if state == :ready do
        assert html =~ "Project-owned builders"
      else
        assert html =~ ~s(id="project-builders-page-state")
      end
    end

    assert length(@covered_states) == 5
  end

  test "renders builder preparation details" do
    html = render_component(&ProjectBuildersPage.project_builders_page/1, assigns(:ready))

    assert html =~ "typescript-node-ubuntu"
    assert html =~ "Dockerfile"
    assert html =~ "sha256:"
    assert html =~ "Registry publication"
    assert html =~ "approved"
    assert html =~ "Verified architectures"
    assert html =~ "scan@sha256:"
  end

  test "renders an explicit empty state" do
    html =
      render_component(
        &ProjectBuildersPage.project_builders_page/1,
        assigns(:ready) |> Map.put(:item_count, 0) |> Map.put(:builders, [])
      )

    assert html =~ "No repository builders were discovered in committed configuration."
  end

  defp assigns(:ready),
    do: %{state: :ready, project_id: "project-1", item_count: 1, builders: [builder()]}

  defp assigns(state), do: Map.put(assigns(:ready), :state, state)

  defp builder do
    %{
      "id" => "builder-1",
      "key" => "typescript-node-ubuntu",
      "display_name" => "TypeScript builder",
      "status" => "prepared",
      "dockerfile_path" => "builders/typescript/Dockerfile",
      "oci_image_digest" =>
        "registry.example/typescript@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "registry_publication" => registry_publication()
    }
  end

  defp registry_publication do
    %{
      "state" => "approved",
      "availability" => "available",
      "immutable_reference" =>
        "registry.example/projects/project/repository-builders/builder@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "architectures" => ["amd64"],
      "sbom" => %{
        "state" => "verified",
        "immutable_reference" =>
          "registry.example/sbom@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
      },
      "provenance" => %{
        "state" => "verified",
        "immutable_reference" =>
          "registry.example/provenance@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
      },
      "scan" => %{
        "state" => "verified",
        "immutable_reference" =>
          "registry.example/scan@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
      },
      "signature" => %{"state" => "not_required"}
    }
  end
end
