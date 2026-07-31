defmodule HephaestusWeb.RPC.Invoke do
  @moduledoc """
  Bounded native-gRPC invocation shared by generated service clients.

  It owns deadlines, mediator metadata, message-size checks, safe query retry,
  channel reset, and transport-error normalization.
  """

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.{Channel, Error, Mediator}

  @default_timeout 5_000
  @default_maximum_request_bytes 1_048_576
  @default_maximum_response_bytes 2_097_152

  @type stub_call :: (GRPC.Channel.t(), struct(), keyword() ->
                        {:ok, struct()} | {:error, GRPC.RPCError.t()})

  @doc "Invokes a generated unary stub with bounded request and response sizes."
  @spec unary(Identity.t(), String.t(), struct(), stub_call(), keyword()) ::
          {:ok, struct()} | {:error, Error.t()}
  def unary(%Identity{} = identity, audience, request, stub_call, options \\ [])
      when is_struct(request) and is_function(stub_call, 3) do
    maximum_request = Keyword.get(options, :maximum_request_bytes, @default_maximum_request_bytes)

    maximum_response =
      Keyword.get(options, :maximum_response_bytes, @default_maximum_response_bytes)

    if encoded_size(request) > maximum_request do
      {:error, Error.local(:size_limit)}
    else
      invoke(identity, audience, request, stub_call, maximum_response, options, 0)
    end
  end

  @doc "Invokes the single identity-bootstrap RPC with verified OIDC metadata."
  def bootstrap_unary(issuer, attributes, audience, request, stub_call, options \\ [])
      when is_binary(issuer) and is_map(attributes) and is_struct(request) and
             is_function(stub_call, 3) do
    maximum_request = Keyword.get(options, :maximum_request_bytes, @default_maximum_request_bytes)

    maximum_response =
      Keyword.get(options, :maximum_response_bytes, @default_maximum_response_bytes)

    if encoded_size(request) > maximum_request do
      {:error, Error.local(:size_limit)}
    else
      invoke_bootstrap(
        issuer,
        attributes,
        audience,
        request,
        stub_call,
        maximum_response,
        options
      )
    end
  end

  defp invoke(identity, audience, request, stub_call, maximum_response, options, attempt) do
    request_id = Keyword.get(options, :request_id)

    metadata_options =
      if request_id, do: [request_id: request_id], else: []

    call_options = [
      metadata: identity |> Mediator.metadata(audience, metadata_options) |> Map.new(),
      timeout: Keyword.get(options, :timeout, @default_timeout)
    ]

    case call(request, stub_call, call_options, options) do
      {:ok, response} ->
        bound_response(response, maximum_response)

      {:error, %GRPC.RPCError{} = rpc_error} ->
        retry_or_error(
          Error.from_rpc(rpc_error),
          identity,
          audience,
          request,
          stub_call,
          maximum_response,
          options,
          attempt
        )

      {:error, _reason} ->
        retry_or_error(
          Error.unavailable(),
          identity,
          audience,
          request,
          stub_call,
          maximum_response,
          options,
          attempt
        )
    end
  end

  defp invoke_bootstrap(
         issuer,
         attributes,
         audience,
         request,
         stub_call,
         maximum_response,
         options
       ) do
    metadata_options = request_id_options(options)

    call_options = [
      metadata:
        issuer
        |> Mediator.bootstrap_metadata(attributes, audience, metadata_options)
        |> Map.new(),
      timeout: Keyword.get(options, :timeout, @default_timeout)
    ]

    case call(request, stub_call, call_options, options) do
      {:ok, response} -> bound_response(response, maximum_response)
      {:error, %GRPC.RPCError{} = rpc_error} -> {:error, Error.from_rpc(rpc_error)}
      {:error, _reason} -> {:error, Error.unavailable()}
    end
  end

  defp call(request, stub_call, call_options, options) do
    case Keyword.fetch(options, :channel_provider) do
      {:ok, channel_provider} ->
        with {:ok, channel} <- channel_provider.() do
          stub_call.(channel, request, call_options)
        end

      :error ->
        Channel.invoke(stub_call, request, call_options)
    end
  end

  defp retry_or_error(
         error,
         identity,
         audience,
         request,
         stub_call,
         maximum_response,
         options,
         attempt
       ) do
    if retry?(error, options, attempt) do
      Keyword.get(options, :channel_reset, &Channel.reset/0).()
      invoke(identity, audience, request, stub_call, maximum_response, options, attempt + 1)
    else
      {:error, error}
    end
  end

  defp request_id_options(options) do
    case Keyword.get(options, :request_id) do
      nil -> []
      request_id -> [request_id: request_id]
    end
  end

  defp bound_response(response, maximum_response) do
    if encoded_size(response) <= maximum_response do
      {:ok, response}
    else
      {:error, Error.local(:size_limit)}
    end
  end

  defp encoded_size(%module{} = message) do
    message
    |> module.encode()
    |> IO.iodata_length()
  end

  defp retry?(%Error{kind: :unavailable}, options, 0),
    do: Keyword.get(options, :retry) == :safe_query

  defp retry?(_error, _options, _attempt), do: false
end
