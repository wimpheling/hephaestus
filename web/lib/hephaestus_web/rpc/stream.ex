defmodule HephaestusWeb.RPC.Stream do
  @moduledoc """
  Bounded native-gRPC server-stream consumption for page-owned watch tasks.

  Every watch owns a dedicated channel. Long-lived streams therefore cannot
  block the shared unary channel, and terminating a supervised watch task also
  tears down its transport process tree.
  """

  alias HephaestusWeb.Identity
  alias HephaestusWeb.RPC.{Error, Mediator}

  @default_maximum_request_bytes 65_536
  @default_maximum_message_bytes 1_048_576

  @type stub_call :: (GRPC.Channel.t(), struct(), keyword() ->
                        {:ok, Enumerable.t()} | {:error, GRPC.RPCError.t()})
  @type consumer :: (struct() -> :cont | :halt)

  @doc "Consumes one generated server stream until completion or consumer halt."
  @spec consume(
          Identity.t(),
          String.t(),
          struct(),
          stub_call(),
          consumer(),
          keyword()
        ) :: :ok | {:error, Error.t()}
  def consume(%Identity{} = identity, audience, request, stub_call, consumer, options \\ [])
      when is_struct(request) and is_function(stub_call, 3) and is_function(consumer, 1) do
    maximum_request =
      Keyword.get(options, :maximum_request_bytes, @default_maximum_request_bytes)

    if encoded_size(request) > maximum_request do
      {:error, Error.local(:size_limit)}
    else
      consume_with_channel(identity, audience, request, stub_call, consumer, options)
    end
  end

  defp consume_with_channel(identity, audience, request, stub_call, consumer, options) do
    channel_provider = Keyword.get(options, :channel_provider, &open_channel/0)
    channel_close = Keyword.get(options, :channel_close, &GRPC.Stub.disconnect/1)

    case channel_provider.() do
      {:ok, %GRPC.Channel{} = channel} ->
        try do
          channel
          |> stub_call.(request, call_options(identity, audience, options))
          |> consume_result(consumer, options)
        catch
          :exit, _reason -> {:error, Error.unavailable()}
        after
          channel_close.(channel)
        end

      {:error, _reason} ->
        {:error, Error.unavailable()}
    end
  end

  defp consume_result({:ok, responses}, consumer, options) do
    maximum_message =
      Keyword.get(options, :maximum_message_bytes, @default_maximum_message_bytes)

    Enum.reduce_while(responses, :ok, fn
      {:ok, response}, :ok when is_struct(response) ->
        consume_message(response, consumer, maximum_message)

      {kind, _metadata}, :ok when kind in [:headers, :trailers] ->
        {:cont, :ok}

      {:error, %GRPC.RPCError{} = error}, :ok ->
        {:halt, {:error, Error.from_rpc(error)}}

      _unexpected, :ok ->
        {:halt, {:error, Error.unavailable()}}
    end)
  rescue
    Protocol.UndefinedError -> {:error, Error.unavailable()}
  end

  defp consume_result({:error, %GRPC.RPCError{} = error}, _consumer, _options),
    do: {:error, Error.from_rpc(error)}

  defp consume_result({:error, _reason}, _consumer, _options),
    do: {:error, Error.unavailable()}

  defp consume_message(response, consumer, maximum_message) do
    if encoded_size(response) > maximum_message do
      {:halt, {:error, Error.local(:size_limit)}}
    else
      case consumer.(response) do
        :cont -> {:cont, :ok}
        :halt -> {:halt, :ok}
      end
    end
  end

  defp call_options(identity, audience, options) do
    [
      metadata: identity |> Mediator.metadata(audience) |> Map.new(),
      timeout: Keyword.get(options, :timeout, :infinity)
    ]
  end

  defp open_channel do
    configuration = Application.fetch_env!(:hephaestus_web, :rpc)

    GRPC.Stub.connect(Keyword.fetch!(configuration, :endpoint),
      adapter: Keyword.get(configuration, :adapter, GRPC.Client.Adapters.Mint),
      adapter_opts: [retry: Keyword.get(configuration, :reconnect_attempts, 5)]
    )
  end

  defp encoded_size(%module{} = message) do
    message
    |> module.encode()
    |> IO.iodata_length()
  end
end
