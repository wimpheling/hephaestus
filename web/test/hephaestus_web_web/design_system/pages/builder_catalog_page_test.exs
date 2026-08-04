defmodule HephaestusWebWeb.DesignSystem.Pages.BuilderCatalogPageTest do
  use ExUnit.Case, async: true

  import Phoenix.LiveViewTest

  alias HephaestusWebWeb.DesignSystem.Pages.BuilderCatalogPage
  alias HephaestusWebWeb.BuilderCatalogState

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
    assert @covered_statuses == BuilderCatalogState.statuses()

    for status <- @covered_statuses do
      state = Map.fetch!(@status_visual_states, status)
      html = render_component(&BuilderCatalogPage.builder_catalog_page/1, assigns(state))

      if state == :ready do
        assert html =~ "Builder image catalog"
      else
        assert html =~ ~s(id="builder-catalog-page-state")
      end
    end

    assert length(@covered_states) == 5
  end

  test "renders digest, policy, toolchain, provenance, and availability metadata" do
    html =
      render_component(
        &BuilderCatalogPage.builder_catalog_page/1,
        assigns(:ready)
      )

    assert html =~ "Builder image catalog"
    assert html =~ "registry.example/rust@sha256:"
    assert html =~ "Rust builder"
    assert html =~ "rust 1.88.0"
    assert html =~ "Disabled"
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
        &BuilderCatalogPage.builder_catalog_page/1,
        assigns(:ready) |> Map.put(:item_count, 0) |> Map.put(:builder_images, [])
      )

    assert html =~ "No builder images are currently available."
  end

  defp assigns(:ready), do: %{state: :ready, item_count: 1, builder_images: [builder_image()]}
  defp assigns(state), do: Map.put(assigns(:ready), :state, state)

  defp builder_image do
    %{
      "id" => "builder-1",
      "key" => "rust",
      "display_name" => "Rust builder",
      "image_reference" =>
        "registry.example/rust@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "toolchains" => [%{"name" => "rust", "version" => "1.88.0"}],
      "architectures" => ["x86_64"],
      "preparation" => "ready",
      "availability" => "available",
      "network_ceiling" => "disabled",
      "max_vcpus" => 2,
      "max_memory_mib" => 512,
      "dependency_policy" => "vendored_offline",
      "provenance" => %{"source" => "attestation://rust"},
      "platform_policy_version" => "build/v1",
      "registry_publication" => registry_publication()
    }
  end

  defp registry_publication do
    %{
      "state" => "approved",
      "availability" => "available",
      "immutable_reference" =>
        "registry.example/platform/builders/rust@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
