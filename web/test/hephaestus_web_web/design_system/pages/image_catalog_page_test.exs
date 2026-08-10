defmodule HephaestusWebWeb.DesignSystem.Pages.ImageCatalogPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.ImageCatalogPage
  alias HephaestusWebWeb.ImageCatalogState

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
    assert @covered_statuses == ImageCatalogState.statuses()

    for status <- @covered_statuses do
      state = Map.fetch!(@status_visual_states, status)
      html = render_component(&ImageCatalogPage.image_catalog_page/1, assigns(state))

      if state == :ready do
        assert html =~ "Images"
      else
        assert html =~ ~s(id="image-catalog-page-state")
      end
    end

    assert length(@covered_states) == 5
  end

  test "renders digest, toolchain, provenance, and availability metadata" do
    html =
      render_component(
        &ImageCatalogPage.image_catalog_page/1,
        assigns(:ready)
      )

    assert html =~ "Images"
    assert html =~ "registry.example/rust@sha256:"
    assert html =~ "Rust Ubuntu"
    assert html =~ "rust 1.88.0"
    assert html =~ "attestation://rust"
    assert html =~ "available"
    assert html =~ "Registry publication"
    assert html =~ "approved"
    assert html =~ "Verified registry architectures"
    assert html =~ "scan@sha256:"
  end

  test "renders an explicit empty catalog state" do
    html =
      render_component(
        &ImageCatalogPage.image_catalog_page/1,
        assigns(:ready) |> Map.put(:item_count, 0) |> Map.put(:images, [])
      )

    assert html =~ "No OCI images are currently available."
  end

  defp assigns(:ready), do: %{state: :ready, item_count: 1, images: [image()]}
  defp assigns(state), do: Map.put(assigns(:ready), :state, state)

  defp image do
    %{
      "id" => "image-1",
      "key" => "rust",
      "display_name" => "Rust Ubuntu",
      "image_reference" =>
        "registry.example/rust@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "toolchains" => [%{"name" => "rust", "version" => "1.88.0"}],
      "architectures" => ["x86_64"],
      "preparation" => "ready",
      "availability" => "available",
      "provenance" => %{"source" => "attestation://rust"},
      "platform_policy_version" => "image/v1",
      "registry_publication" => registry_publication()
    }
  end

  defp registry_publication do
    %{
      "state" => "approved",
      "availability" => "available",
      "immutable_reference" =>
        "registry.example/platform/images/rust@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
