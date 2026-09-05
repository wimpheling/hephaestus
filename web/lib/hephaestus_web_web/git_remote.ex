defmodule HephaestusWebWeb.GitRemote do
  @moduledoc false

  @type repository :: %{required(String.t()) => String.t()}

  @spec url(String.t()) :: String.t()
  def url(repository_id) when is_binary(repository_id) do
    configuration()
    |> Keyword.fetch!(:origin)
    |> url(repository_id)
  end

  @spec url(String.t(), String.t()) :: String.t()
  def url(origin, repository_id) when is_binary(origin) and is_binary(repository_id) do
    parsed = parse_origin!(origin)

    parsed
    |> Map.put(:path, append_path(origin_path(parsed), repository_id))
    |> URI.to_string()
  end

  @spec clone_command(repository(), String.t()) :: String.t()
  def clone_command(repository, remote_url) when is_map(repository) and is_binary(remote_url) do
    "git clone #{pat_remote_url(remote_url)} #{directory_name(repository["name"])}"
  end

  defp configuration, do: Application.fetch_env!(:hephaestus_web, :git_http)

  defp parse_origin!(origin) do
    parsed = URI.parse(origin)

    case {parsed.scheme, parsed.host, parsed.userinfo, parsed.query, parsed.fragment} do
      {scheme, host, nil, nil, nil}
      when scheme in ["http", "https"] and is_binary(host) and host != "" ->
        parsed

      _invalid_origin ->
        raise ArgumentError,
              "HEPHAESTUS_GIT_HTTP_ORIGIN must be an absolute HTTP(S) origin without credentials, query, or fragment"
    end
  end

  defp origin_path(%URI{path: nil}), do: ""
  defp origin_path(%URI{path: path}), do: String.trim_trailing(path, "/")

  defp append_path("", repository_id), do: "/#{repository_id}"
  defp append_path(path, repository_id), do: "#{path}/#{repository_id}"

  # `heph-pat` is a public Basic-auth discriminator, not bearer material. The
  # server still prompts for the personal access token and it is never persisted
  # in a remote URL or the copied command.
  defp pat_remote_url(remote_url) do
    remote_url
    |> URI.parse()
    |> Map.put(:userinfo, "heph-pat")
    |> URI.to_string()
  end

  defp directory_name(name) when is_binary(name) do
    name
    |> String.downcase()
    |> String.replace(~r/[^a-z0-9._-]+/, "-")
    |> String.replace(~r/[-._]{2,}/, "-")
    |> String.replace(~r/^[.-]+|[.-]+$/, "")
    |> case do
      "" -> "repository"
      "." -> "repository"
      ".." -> "repository"
      directory -> directory
    end
  end

  defp directory_name(_missing_name), do: "repository"
end
