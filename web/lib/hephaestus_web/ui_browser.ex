defmodule HephaestusWeb.UIBrowser do
  @moduledoc """
  Builds one bounded generation-origin bootstrap URL for an installed UI.

  The fragment is exactly the one-time 32-byte handoff bearer. Callers must
  consume the returned URL through a one-shot browser event and must not put it
  in LiveView assigns, durable state, logs, or history.
  """

  @uuid_pattern ~r/\A[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\z/i
  @label_pattern ~r/\A[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\z/
  @route_pattern ~r{\A[A-Za-z0-9._~-]+(?:/[A-Za-z0-9._~-]+)*\z}
  @bootstrap_path "/_heph/bootstrap"

  @type projection :: %{optional(String.t()) => term()} | %{optional(atom()) => term()}
  @type error :: :invalid_projection | :invalid_secret | :invalid_theme | :unavailable

  @doc "Returns a fresh 32-byte handoff bearer for one RPC request."
  @spec new_handoff_secret() :: binary()
  def new_handoff_secret, do: :crypto.strong_rand_bytes(32)

  @doc "Builds a deterministic HTTPS bootstrap URL from safe navigation metadata."
  @spec bootstrap_url(projection(), binary(), String.t(), keyword()) ::
          {:ok, String.t()} | {:error, error()}
  def bootstrap_url(projection, secret, theme, options \\ []) do
    with {:ok, generation_id} <- field(projection, "generation_id"),
         {:ok, route_base} <- field(projection, "route_base"),
         :ok <- validate_generation(generation_id),
         :ok <- validate_route(route_base),
         :ok <- validate_secret(secret),
         :ok <- validate_theme(theme),
         {:ok, config} <- configuration(options),
         :ok <- validate_namespace(config.namespace, config.platform_host),
         {:ok, port} <- validate_port(config.port) do
      simple_generation = generation_id |> String.downcase() |> String.replace("-", "")
      host = "g-" <> simple_generation <> "." <> config.namespace
      authority = if port == 443, do: host, else: host <> ":#{port}"
      fragment = Base.url_encode64(secret, padding: false)
      query = URI.encode_query(%{"heph_theme" => theme})

      {:ok, "https://#{authority}#{@bootstrap_path}?#{query}##{fragment}"}
    end
  end

  @doc false
  @spec validate_projection(projection()) :: {:ok, map()} | {:error, :invalid_projection}
  def validate_projection(projection) do
    with {:ok, generation_id} <- field(projection, "generation_id"),
         {:ok, route_base} <- field(projection, "route_base"),
         :ok <- validate_generation(generation_id),
         :ok <- validate_route(route_base) do
      {:ok, %{"generation_id" => generation_id, "route_base" => route_base}}
    end
  end

  defp configuration(options) do
    configured = Application.get_env(:hephaestus_web, :ui_browser, [])
    namespace = Keyword.get(options, :namespace, Keyword.get(configured, :namespace))
    platform_host = Keyword.get(options, :platform_host, Keyword.get(configured, :platform_host))
    port = Keyword.get(options, :port, Keyword.get(configured, :port, 443))

    if is_binary(namespace) and is_binary(platform_host) do
      {:ok,
       %{
         namespace: String.downcase(namespace),
         platform_host: String.downcase(platform_host),
         port: port
       }}
    else
      {:error, :unavailable}
    end
  end

  defp field(projection, "generation_id") when is_map(projection) do
    case Map.get(projection, "generation_id") || Map.get(projection, :generation_id) do
      value when is_binary(value) and value != "" -> {:ok, value}
      _invalid -> {:error, :invalid_projection}
    end
  end

  defp field(projection, "route_base") when is_map(projection) do
    case Map.get(projection, "route_base") || Map.get(projection, :route_base) do
      value when is_binary(value) and value != "" -> {:ok, value}
      _invalid -> {:error, :invalid_projection}
    end
  end

  defp field(_projection, _key), do: {:error, :invalid_projection}

  defp validate_generation(value) do
    if Regex.match?(@uuid_pattern, value), do: :ok, else: {:error, :invalid_projection}
  end

  defp validate_route(value) do
    valid =
      byte_size(value) in 1..256 and Regex.match?(@route_pattern, value) and
        not Enum.any?(String.split(value, "/"), &(&1 in [".", ".."]))

    if valid, do: :ok, else: {:error, :invalid_projection}
  end

  defp validate_secret(value) when is_binary(value) and byte_size(value) == 32, do: :ok
  defp validate_secret(_value), do: {:error, :invalid_secret}

  defp validate_theme(theme) when theme in ["light", "dark"], do: :ok
  defp validate_theme(_theme), do: {:error, :invalid_theme}

  defp validate_namespace(namespace, platform_host) do
    # Rust's UiNamespace reserves room for `g-<32 hex>.` so the resulting
    # generation authority always remains within the 253-byte DNS limit.
    labels_valid =
      byte_size(namespace) in 1..218 and
        not String.ends_with?(namespace, ".") and
        Enum.all?(String.split(namespace, "."), &Regex.match?(@label_pattern, &1))

    strict_subdomain =
      namespace != platform_host and
        String.ends_with?(namespace, "." <> platform_host)

    if labels_valid and valid_hostname?(platform_host) and strict_subdomain do
      :ok
    else
      {:error, :unavailable}
    end
  end

  defp valid_hostname?(hostname) do
    byte_size(hostname) in 1..253 and
      not String.ends_with?(hostname, ".") and
      Enum.all?(String.split(hostname, "."), &Regex.match?(@label_pattern, &1))
  end

  defp validate_port(nil), do: {:ok, 443}
  defp validate_port(port) when is_integer(port) and port in 1..65_535, do: {:ok, port}

  defp validate_port(port) when is_binary(port) do
    case Integer.parse(port) do
      {value, ""} ->
        if Integer.to_string(value) == port,
          do: validate_port(value),
          else: {:error, :unavailable}

      _invalid ->
        {:error, :unavailable}
    end
  end

  defp validate_port(_port), do: {:error, :unavailable}
end
