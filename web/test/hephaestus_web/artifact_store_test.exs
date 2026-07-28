defmodule HephaestusWeb.ArtifactStoreTest do
  use ExUnit.Case, async: false

  alias HephaestusWeb.ArtifactStore

  test "reads only bounded files beneath the configured artifact root" do
    root =
      Path.join(System.tmp_dir!(), "hephaestus-artifacts-#{System.unique_integer([:positive])}")

    File.mkdir_p!(Path.join(root, "run"))
    File.write!(Path.join(root, "run/patch.patch"), "diff --git a/a b/a\n")
    previous = Application.get_env(:hephaestus_web, :artifact_root)
    Application.put_env(:hephaestus_web, :artifact_root, root)

    on_exit(fn ->
      Application.put_env(:hephaestus_web, :artifact_root, previous)
      File.rm_rf!(root)
    end)

    assert {:ok, "diff --git a/a b/a\n"} = ArtifactStore.read_preview("run/patch.patch")
    assert {:error, :unavailable} = ArtifactStore.read_preview("../outside")
  end
end
