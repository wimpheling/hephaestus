defmodule HephaestusWeb.CommandClient do
  @moduledoc """
  Sends privileged browser commands to the trusted Rust boundary.

  Secret values exist only in the transient function argument and encoded
  request body. Responses are value-free, and callers must never assign the
  submitted value to a socket.
  """

  alias HephaestusWeb.Identity

  def execute(%Identity{} = identity, operation, attributes) when is_map(attributes) do
    request_id = Ecto.UUID.generate()
    configuration = Application.fetch_env!(:hephaestus_web, :internal_commands)

    body =
      attributes
      |> Map.put("operation", operation)
      |> Map.put("actor_id", identity.user_id)
      |> Map.put("request_id", request_id)

    request_options = [
      url: Keyword.fetch!(configuration, :url),
      auth: {:bearer, Keyword.fetch!(configuration, :token)},
      json: body,
      receive_timeout: 15_000,
      retry: false
    ]

    request_options =
      case Keyword.get(configuration, :plug) do
        nil -> request_options
        plug -> Keyword.put(request_options, :plug, plug)
      end

    case Req.post(request_options) do
      {:ok, %{status: status, body: response}} when status in 200..299 ->
        {:ok, response}

      {:ok, %{status: status}} ->
        {:error, {:rejected, status}}

      {:error, reason} ->
        {:error, {:unavailable, reason}}
    end
  end
end
