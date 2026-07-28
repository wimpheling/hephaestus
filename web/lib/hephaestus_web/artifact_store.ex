defmodule HephaestusWeb.ArtifactStore do
  @moduledoc """
  Read-only access to Phase 4 artifacts beneath one configured canonical root.
  """

  @maximum_preview_bytes 2_000_000

  def read_preview(storage_key) when is_binary(storage_key) do
    root =
      :hephaestus_web
      |> Application.fetch_env!(:artifact_root)
      |> Path.expand()

    candidate = Path.expand(storage_key, root)

    with true <- within?(candidate, root),
         {:ok, stat} <- File.stat(candidate),
         true <- stat.type == :regular and stat.size <= @maximum_preview_bytes,
         {:ok, contents} <- File.read(candidate) do
      {:ok, contents}
    else
      _ -> {:error, :unavailable}
    end
  end

  def read_preview(_storage_key), do: {:error, :unavailable}

  defp within?(candidate, root) do
    candidate == root or String.starts_with?(candidate, root <> "/")
  end
end
