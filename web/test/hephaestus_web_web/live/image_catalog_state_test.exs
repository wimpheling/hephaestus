defmodule HephaestusWebWeb.ImageCatalogStateTest do
  use ExUnit.Case, async: true

  alias HephaestusWebWeb.ImageCatalogState

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
    assert @covered_statuses == ImageCatalogState.statuses()
  end

  test "accepts the current generation and ignores stale snapshots" do
    state = ImageCatalogState.new(%{})
    {loading, effects} = ImageCatalogState.reduce(state, :load)
    assert loading.status == :loading
    assert effects == [:load]

    {ready, []} =
      ImageCatalogState.reduce(
        loading,
        {:loaded, loading.stream_generation, [image()]}
      )

    assert ready.status == :ready
    assert ready.data.images == [image()]

    assert ImageCatalogState.reduce(ready, {:loaded, 3, []}) == {ready, []}
  end

  test "exposes a deterministic presentation for an empty catalog" do
    presentation = ImageCatalogState.present(ImageCatalogState.new(%{}))
    assert presentation.state == :loading
    assert presentation.images == []
    assert presentation.item_count == 0

    assert Map.keys(Map.from_struct(ImageCatalogState.new(%{}))) |> Enum.sort() ==
             [:cursor, :data, :error, :form, :status, :stream_generation]
  end

  defp image do
    %{
      "id" => "image-1",
      "key" => "rust",
      "display_name" => "Rust image",
      "image_reference" =>
        "registry.example/rust@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "toolchains" => [%{"name" => "rust", "version" => "1.88.0"}],
      "architectures" => ["x86_64"],
      "preparation" => "ready",
      "availability" => "available",
      "provenance" => %{"source" => "attestation://rust"},
      "platform_policy_version" => "image/v1"
    }
  end
end
