defmodule HephaestusWebWeb.BuilderCatalogStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.BuilderCatalogState

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

  test "declares the complete lifecycle" do
    assert @covered_statuses == BuilderCatalogState.statuses()
  end

  test "accepts the current generation and ignores stale snapshots" do
    state = BuilderCatalogState.new(%{})
    {loading, effects} = BuilderCatalogState.reduce(state, :load)
    assert loading.status == :loading
    assert effects == [:load]

    {ready, []} =
      BuilderCatalogState.reduce(
        loading,
        {:loaded, loading.stream_generation, [builder_image()]}
      )

    assert ready.status == :ready
    assert ready.data.builder_images == [builder_image()]

    assert BuilderCatalogState.reduce(ready, {:loaded, 3, []}) == {ready, []}
  end

  test "exposes a deterministic presentation for an empty catalog" do
    presentation = BuilderCatalogState.present(BuilderCatalogState.new(%{}))
    assert presentation.state == :loading
    assert presentation.builder_images == []
    assert presentation.item_count == 0

    assert Map.keys(Map.from_struct(BuilderCatalogState.new(%{}))) |> Enum.sort() ==
             [:cursor, :data, :error, :form, :status, :stream_generation]
  end

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
      "platform_policy_version" => "build/v1"
    }
  end
end
