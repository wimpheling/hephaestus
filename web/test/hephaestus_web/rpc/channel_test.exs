defmodule HephaestusWeb.RPC.ChannelTest do
  use ExUnit.Case, async: true

  alias HephaestusWeb.RPC.Channel

  defmodule Connector do
    def connect(endpoint, options) do
      send(endpoint, {:connect, options})
      {:ok, %GRPC.Channel{host: "rpc.test", port: 443}}
    end

    def disconnect(channel), do: {:ok, channel}
  end

  defmodule BlockingDisconnectConnector do
    def connect(endpoint, _options) do
      send(endpoint, :connect)
      {:ok, %GRPC.Channel{host: endpoint, port: 443}}
    end

    def disconnect(channel) do
      send(channel.host, {:disconnect_started, self()})

      receive do
        :finish_disconnect -> :ok
      end
    end
  end

  test "connects lazily, reuses the channel, and resets it" do
    configuration = [endpoint: self(), reconnect_attempts: 2]

    pid =
      start_supervised!(
        {Channel, name: nil, configuration: configuration, connector: Connector},
        id: make_ref()
      )

    assert {:ok, %GRPC.Channel{} = first} = Channel.get(pid)

    assert_receive {:connect,
                    [
                      adapter: GRPC.Client.Adapters.Mint,
                      adapter_opts: [retry: 2]
                    ]}

    assert {:ok, ^first} = Channel.get(pid)
    refute_receive {:connect, _options}

    assert :ok = Channel.reset(pid)
    assert {:ok, %GRPC.Channel{}} = Channel.get(pid)

    assert_receive {:connect, _options}
  end

  test "a canceled caller cannot poison the supervised channel" do
    configuration = [endpoint: self(), reconnect_attempts: 2]

    channel =
      start_supervised!(
        {Channel, name: nil, configuration: configuration, connector: Connector},
        id: make_ref()
      )

    test_process = self()

    blocked_stub = fn _grpc_channel, _request, _options ->
      send(test_process, {:stub_started, self()})

      receive do
        :release -> {:ok, %Google.Protobuf.Empty{}}
      end
    end

    caller =
      spawn(fn ->
        Channel.invoke(blocked_stub, %Google.Protobuf.Empty{}, [], channel)
      end)

    assert_receive {:stub_started, ^channel}
    Process.exit(caller, :kill)
    send(channel, :release)

    assert {:ok, %Google.Protobuf.StringValue{value: "next"}} =
             Channel.invoke(
               fn _grpc_channel, _request, _options ->
                 {:ok, %Google.Protobuf.StringValue{value: "next"}}
               end,
               %Google.Protobuf.Empty{},
               [],
               channel
             )

    assert Process.alive?(channel)
  end

  test "reset makes a fresh channel available without waiting for stale disconnect" do
    configuration = [endpoint: self(), reconnect_attempts: 2]

    channel =
      start_supervised!(
        {Channel,
         name: nil, configuration: configuration, connector: BlockingDisconnectConnector},
        id: make_ref()
      )

    assert {:ok, %GRPC.Channel{}} = Channel.get(channel)
    assert_receive :connect

    assert :ok = Channel.reset(channel)
    assert {:ok, %GRPC.Channel{}} = Channel.get(channel)
    assert_receive :connect
    assert_receive {:disconnect_started, disconnect_pid}

    send(disconnect_pid, :finish_disconnect)
  end
end
