defmodule HephaestusWeb.RPC.Channel do
  @moduledoc """
  Supervised, lazily connected native gRPC channel.

  Keeping connection ownership here gives concurrent page effects one reusable
  channel while allowing the transport adapter to reconnect after a Rust
  service restart.
  """

  use GenServer

  @type connector :: module()

  @spec start_link(keyword()) :: GenServer.on_start()
  def start_link(options \\ []) do
    name = Keyword.get(options, :name, __MODULE__)

    case name do
      nil -> GenServer.start_link(__MODULE__, options)
      registered_name -> GenServer.start_link(__MODULE__, options, name: registered_name)
    end
  end

  @doc "Returns the shared channel, establishing it on its first use."
  @spec get(GenServer.server()) :: {:ok, GRPC.Channel.t()} | {:error, term()}
  def get(server \\ __MODULE__), do: GenServer.call(server, :get)

  @doc "Drops a failed channel so the next effect establishes a fresh one."
  @spec reset(GenServer.server()) :: :ok
  def reset(server \\ __MODULE__) do
    GenServer.cast(server, :reset)
  end

  @doc "Runs one generated stub call under the supervised channel owner."
  @spec invoke(
          (GRPC.Channel.t(), struct(), keyword() -> term()),
          struct(),
          keyword(),
          GenServer.server()
        ) ::
          term()
  def invoke(stub_call, request, options, server \\ __MODULE__)
      when is_function(stub_call, 3) and is_struct(request) do
    GenServer.call(server, {:invoke, stub_call, request, options}, :infinity)
  end

  @impl true
  def init(options) do
    {:ok,
     %{
       channel: nil,
       configuration: Keyword.get(options, :configuration),
       connector: Keyword.get(options, :connector, GRPC.Stub)
     }}
  end

  @impl true
  def handle_call(:get, _from, %{channel: %GRPC.Channel{} = channel} = state) do
    {:reply, {:ok, channel}, state}
  end

  def handle_call(:get, _from, state) do
    case connect(state) do
      {:ok, channel, state} -> {:reply, {:ok, channel}, state}
      {:error, reason} -> {:reply, {:error, reason}, state}
    end
  end

  def handle_call({:invoke, stub_call, request, options}, _from, state) do
    case connect(state) do
      {:ok, channel, state} ->
        try do
          {:reply, stub_call.(channel, request, options), state}
        catch
          :exit, _reason ->
            disconnect(state)
            {:reply, {:error, :transport_exit}, %{state | channel: nil}}
        end

      {:error, reason} ->
        {:reply, {:error, reason}, state}
    end
  end

  defp connect(%{channel: %GRPC.Channel{} = channel} = state),
    do: {:ok, channel, state}

  defp connect(state) do
    configuration = state.configuration || Application.fetch_env!(:hephaestus_web, :rpc)
    endpoint = Keyword.fetch!(configuration, :endpoint)

    options = [
      adapter: Keyword.get(configuration, :adapter, GRPC.Client.Adapters.Mint),
      adapter_opts: [retry: Keyword.get(configuration, :reconnect_attempts, 5)]
    ]

    case state.connector.connect(endpoint, options) do
      {:ok, %GRPC.Channel{} = channel} ->
        {:ok, channel, %{state | channel: channel}}

      {:error, reason} ->
        {:error, reason}
    end
  end

  @impl true
  def handle_cast(:reset, state) do
    disconnect_async(state)
    {:noreply, %{state | channel: nil}}
  end

  @impl true
  def handle_info({:EXIT, _pid, :normal}, state), do: {:noreply, state}

  def handle_info({:EXIT, _pid, _reason}, state) do
    disconnect(state)
    {:noreply, %{state | channel: nil}}
  end

  @impl true
  def terminate(_reason, state) do
    disconnect(state)
    :ok
  end

  defp disconnect(%{channel: %GRPC.Channel{} = channel, connector: connector}) do
    connector.disconnect(channel)
    :ok
  end

  defp disconnect(_state), do: :ok

  # Graceful Mint shutdown may wait for a failed peer. Channel invalidation is
  # on the page retry path, so publish the empty slot before closing the stale
  # connection and let the next caller connect immediately.
  defp disconnect_async(%{channel: %GRPC.Channel{} = channel, connector: connector}) do
    Task.start(fn -> connector.disconnect(channel) end)
    :ok
  end

  defp disconnect_async(_state), do: :ok
end
